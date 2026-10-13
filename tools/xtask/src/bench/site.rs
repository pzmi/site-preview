//! The generated benchmark site: 1,000 served files in nested directories beside an ignored
//! `node_modules` of 10,000 files, in a private temporary directory removed on drop.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Served files, `index.html` included (plan section 13: "<= 1,000 files").
pub const SERVED_FILES: usize = 1_000;
/// Files below `node_modules`, which the server never serves or watches (plan section 5).
pub const IGNORED_FILES: usize = 10_000;
/// The file the benchmarks rewrite to cause a reload.
pub const SAVE_TARGET: &str = "section0/page0/item0.html";

/// Relative paths of the `total` served files: `index.html`, then `sectionA/pageB/itemC.ext`
/// with 10 sections of 10 pages, so the tree is three levels deep. The extension cycles over
/// `html`, `css` and `js`.
pub fn served_layout(total: usize) -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from("index.html")];
    let mut index = 0;
    while files.len() < total {
        let (section, page, item) = (index / 100 % 10, index / 10 % 10, index % 10);
        let ext = ["html", "css", "js"][index % 3];
        // Item 0 of every page is html, so `SAVE_TARGET` exists and is a page.
        let ext = if item == 0 { "html" } else { ext };
        files.push(PathBuf::from(format!(
            "section{section}/page{page}/item{item}.{ext}"
        )));
        index += 1;
    }
    files
}

/// Relative paths of the `total` ignored files: `node_modules/pkgA/lib/fB.js`, 100 per package.
pub fn ignored_layout(total: usize) -> Vec<PathBuf> {
    (0..total)
        .map(|i| PathBuf::from(format!("node_modules/pkg{}/lib/f{}.js", i / 100, i % 100)))
        .collect()
}

/// The synthetic content of a served file.
pub fn content(path: &Path) -> String {
    let name = path.display();
    match path.extension().and_then(|e| e.to_str()) {
        Some("css") => format!("/* {name} */\nbody {{ margin: 0; }}\n"),
        Some("js") => format!("// {name}\nconsole.log('synthetic');\n"),
        _ => format!(
            "<!doctype html>\n<html><head><title>{name}</title></head>\n\
             <body><h1>{name}</h1><p>Synthetic benchmark page.</p></body></html>\n"
        ),
    }
}

/// A generated site in a fresh directory that is removed when this is dropped.
pub struct BenchSite {
    temp: PathBuf,
    root: PathBuf,
}

impl BenchSite {
    /// Creates the temporary directory and generates both trees.
    pub fn generate() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let temp =
            std::env::temp_dir().join(format!("site-preview-bench-{}-{nanos}", std::process::id()));
        // `create_dir` (not `_all`) refuses a name that already exists, so a pre-planted
        // directory or symlink is never written through.
        fs::create_dir(&temp).map_err(|e| format!("cannot create {}: {e}", temp.display()))?;
        let site = Self {
            root: temp.join("site"),
            temp,
        };
        for path in served_layout(SERVED_FILES)
            .into_iter()
            .chain(ignored_layout(IGNORED_FILES))
        {
            let full = site.root.join(&path);
            let parent = full.parent().ok_or("a file without a parent")?;
            fs::create_dir_all(parent)
                .and_then(|()| fs::write(&full, content(&path)))
                .map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        }
        Ok(site)
    }

    /// The directory to serve.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory that holds the site, for the filesystem probe.
    pub fn temp_dir(&self) -> &Path {
        &self.temp
    }

    /// Rewrites the save target with content that differs on every `revision`.
    pub fn save(&self, revision: usize) -> Result<(), String> {
        let path = self.root.join(SAVE_TARGET);
        let body = format!(
            "<!doctype html>\n<html><head><title>save</title></head>\n\
             <body><p>revision {revision}</p></body></html>\n"
        );
        fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

impl Drop for BenchSite {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn should_list_exact_count_of_unique_files_when_layout_is_built() {
        // given
        let total = SERVED_FILES;

        // when
        let files = served_layout(total);

        // then
        let unique: HashSet<_> = files.iter().collect();
        assert_eq!(files.len(), total);
        assert_eq!(unique.len(), total);
    }

    #[test]
    fn should_nest_three_levels_when_layout_has_more_than_the_index() {
        // given
        let files = served_layout(SERVED_FILES);

        // when
        let depths: HashSet<usize> = files.iter().map(|f| f.components().count()).collect();

        // then
        assert_eq!(depths, HashSet::from([1, 3]));
        assert!(files.contains(&PathBuf::from(SAVE_TARGET)));
    }

    #[test]
    fn should_hold_only_ignored_paths_when_node_modules_layout_is_built() {
        // given
        let total = IGNORED_FILES;

        // when
        let files = ignored_layout(total);

        // then
        let unique: HashSet<_> = files.iter().collect();
        assert_eq!(unique.len(), total);
        assert!(files.iter().all(|f| f.starts_with("node_modules")));
    }

    #[test]
    fn should_generate_and_remove_tree_when_site_is_dropped() {
        // given
        let site = BenchSite::generate().unwrap();
        let temp = site.temp_dir().to_path_buf();

        // when
        let saved = site.save(1).and_then(|()| site.save(2));
        let body = fs::read_to_string(site.root().join(SAVE_TARGET)).unwrap();
        let has_index = site.root().join("index.html").is_file();
        drop(site);

        // then
        assert!(saved.is_ok());
        assert!(body.contains("revision 2"));
        assert!(has_index);
        assert!(!temp.exists());
    }
}
