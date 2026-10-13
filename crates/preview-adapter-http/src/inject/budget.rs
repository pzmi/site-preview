//! The memory budget for HTML held whole in memory (ADR 0046).
//!
//! An HTML page of at most 8 MiB is read whole so that its encoding can be checked and the
//! client script inserted. Without a bound, every concurrent request holds its own copy until
//! the client has read it, so stalled clients could pin gigabytes. Each such request first
//! reserves [`charge`] bytes of an [`HtmlBudget`] and keeps the reservation until its body has
//! been sent or dropped; a request that does not fit waits for one to be released.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_stream::Stream;

/// The default budget: 128 MiB, room for eight 8 MiB pages being rewritten at once and many
/// more small ones.
pub const DEFAULT_HTML_BUDGET_BYTES: u64 = 128 * 1024 * 1024;

/// Reservations are counted in KiB, so the largest charge fits a `u32` permit count.
const UNIT: u64 = 1024;

/// Bytes charged on top of twice the page: the script tag and the parser's own buffers.
const SLACK: u64 = 64 * 1024;

/// Bytes sent per body chunk. Each chunk is a copy, so the connection's write buffer never
/// holds a slice that keeps the whole page alive after the reservation is released.
const CHUNK: usize = 64 * 1024;

/// A shared byte budget for whole-file HTML buffers. Clones share the same budget.
///
/// The bound it gives: at any moment, the HTML bytes held by this adapter for pages being
/// read, rewritten or sent total at most the budget (the original and the rewritten copy are
/// both charged, see [`charge`]). Streamed bodies (files over 8 MiB, every non-HTML file) are
/// outside it: they hold one 64 KiB chunk each. Pinned by
/// `should_hold_budget_until_body_is_consumed_when_html_served` in `tests/inject.rs` and the
/// tests below.
#[derive(Debug, Clone)]
pub struct HtmlBudget {
    permits: Arc<Semaphore>,
    total: u32,
}

impl HtmlBudget {
    /// A budget of `bytes`, rounded down to whole KiB, at least 1 KiB.
    #[must_use]
    pub fn new(bytes: u64) -> Self {
        let max = u64::try_from(Semaphore::MAX_PERMITS)
            .unwrap_or(u64::MAX)
            .min(u64::from(u32::MAX));
        let total = u32::try_from((bytes / UNIT).clamp(1, max)).unwrap_or(u32::MAX);
        Self {
            permits: Arc::new(Semaphore::new(total as usize)),
            total,
        }
    }

    /// Bytes not reserved right now.
    #[must_use]
    pub fn available_bytes(&self) -> u64 {
        self.permits.available_permits() as u64 * UNIT
    }

    /// Waits until `bytes` (capped at the whole budget, so one page always fits eventually)
    /// can be reserved. `None` only if the budget was closed, which never happens.
    pub(crate) async fn reserve(&self, bytes: u64) -> Option<OwnedSemaphorePermit> {
        let units = u32::try_from(bytes.div_ceil(UNIT))
            .unwrap_or(u32::MAX)
            .clamp(1, self.total);
        Arc::clone(&self.permits)
            .acquire_many_owned(units)
            .await
            .ok()
    }
}

impl Default for HtmlBudget {
    fn default() -> Self {
        Self::new(DEFAULT_HTML_BUDGET_BYTES)
    }
}

/// What reading and rewriting a page of `len` bytes is charged: the original, the rewritten
/// copy and [`SLACK`].
pub(crate) const fn charge(len: u64) -> u64 {
    len.saturating_mul(2).saturating_add(SLACK)
}

/// A body sending `bytes` that keeps `permit` until the last chunk has been handed over or
/// the body is dropped (a client gone, the server shutting down).
pub(crate) fn buffered_body(bytes: Vec<u8>, permit: OwnedSemaphorePermit) -> Body {
    Body::from_stream(Buffered {
        bytes,
        offset: 0,
        permit: Some(permit),
    })
}

struct Buffered {
    bytes: Vec<u8>,
    offset: usize,
    permit: Option<OwnedSemaphorePermit>,
}

impl Stream for Buffered {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let Some(chunk) = this
            .bytes
            .get(this.offset..)
            .filter(|rest| !rest.is_empty())
        else {
            this.bytes = Vec::new();
            this.permit = None;
            return Poll::Ready(None);
        };
        let chunk = &chunk[..chunk.len().min(CHUNK)];
        this.offset += chunk.len();
        Poll::Ready(Some(Ok(Bytes::copy_from_slice(chunk))))
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::Waker;

    use http_body_util::BodyExt;

    use super::*;

    #[tokio::test]
    async fn should_wait_for_release_when_budget_exhausted() {
        // given
        let budget = HtmlBudget::new(4 * UNIT);
        let held = budget.reserve(4 * UNIT).await.expect("fits");
        let mut waiting = Box::pin(budget.reserve(UNIT));
        let mut cx = Context::from_waker(Waker::noop());

        // when
        let before = waiting.as_mut().poll(&mut cx).is_pending();
        drop(held);
        let after = waiting.as_mut().poll(&mut cx).is_ready();

        // then
        assert!(before, "a reservation beyond the budget must wait");
        assert!(after, "a release must let the waiting reservation through");
    }

    #[tokio::test]
    async fn should_release_reservation_when_body_sent_or_dropped() {
        // given
        let budget = HtmlBudget::new(64 * UNIT);
        let page = vec![b'x'; 3 * CHUNK + 5];
        let sent = buffered_body(page.clone(), budget.reserve(32 * UNIT).await.expect("fits"));
        let dropped = buffered_body(page.clone(), budget.reserve(32 * UNIT).await.expect("fits"));
        assert_eq!(budget.available_bytes(), 0);

        // when
        let body = sent.collect().await.expect("infallible").to_bytes();
        let after_send = budget.available_bytes();
        drop(dropped);

        // then
        assert_eq!(body.as_ref(), page.as_slice());
        assert_eq!(after_send, 32 * UNIT);
        assert_eq!(budget.available_bytes(), 64 * UNIT);
    }

    #[tokio::test]
    async fn should_cap_reservation_when_charge_exceeds_budget() {
        // given
        let budget = HtmlBudget::new(8 * UNIT);

        // when
        let permit = budget.reserve(charge(1024 * UNIT)).await;

        // then
        assert!(permit.is_some());
        assert_eq!(budget.available_bytes(), 0);
    }

    #[test]
    fn should_hold_128_mib_when_budget_is_the_default() {
        // given
        let budget = HtmlBudget::default();

        // when
        let available = budget.available_bytes();

        // then
        assert_eq!(available, 128 * 1024 * 1024);
        assert_eq!(DEFAULT_HTML_BUDGET_BYTES, available);
    }

    #[tokio::test]
    async fn should_send_64_kib_chunks_when_page_is_larger() {
        // given
        let budget = HtmlBudget::new(1024 * UNIT);
        let permit = budget.reserve(UNIT).await.expect("fits");
        let mut body = buffered_body(vec![b'x'; 2 * 64 * 1024 + 1], permit);

        // when
        let mut sizes = Vec::new();
        while let Some(frame) = body.frame().await {
            sizes.push(frame.expect("infallible").into_data().expect("data").len());
        }

        // then
        assert_eq!(sizes, [64 * 1024, 64 * 1024, 1]);
    }
}
