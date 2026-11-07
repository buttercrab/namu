//! A crawler: the data collection NAMU is for, on a site that misbehaves as real ones do.
//!
//! The tasks are Python (`../tasks`): fetching is I/O, and Python is where crawling code lives.

#![allow(dead_code)]

use namu::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub html: String,
}

/// The first frontier: the seed alone.
#[task(single)]
pub fn start(seed: String) -> Result<Vec<String>>;

/// A page; a server that says to wait, or is briefly down, is asked again. Anything else fails
/// the call.
#[task(single)]
pub fn fetch(url: String) -> Result<Page>;

/// The links on a page, to the same site.
#[task(single)]
pub fn links(page: Page) -> Result<Vec<String>>;

/// The links found on a level's pages that no earlier level saw.
#[task(single)]
pub fn unseen(found: Vec<Vec<String>>, seen: Vec<String>) -> Result<Vec<String>>;

#[task(single)]
pub fn union(seen: Vec<String>, new: Vec<String>) -> Result<Vec<String>>;

#[task(single)]
pub fn size(urls: Vec<String>) -> Result<i32>;

/// Crawls `depth` levels of links from `seed`, and counts the URLs it found.
///
/// - A level's pages are fetched at once, a branch each, across the workers.
/// - A page that cannot be fetched (a dead link) fails its own branch: it is dropped, and recorded
///   in the run's failures; the crawl goes on.
/// - Every page is fetched once: a level's links are checked against everything seen before.
#[workflow(id = "crawl")]
pub fn crawl(seed: String, depth: i32) -> i32 {
    let mut frontier = start(seed);
    let mut seen = frontier;
    let mut level = 0;
    while level < depth {
        let url = each(frontier);
        let found = links(fetch(url));
        let next = unseen(found, seen);
        seen = union(seen, next);
        frontier = next;
        level += 1;
    }
    size(seen)
}
