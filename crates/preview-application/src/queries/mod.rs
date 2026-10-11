//! Queries: use cases that observe state without changing a domain decision.
//!
//! A query may open content through a port and read the current state, but it never advances
//! a revision or emits a [`DomainEvent`](crate::events::DomainEvent).
