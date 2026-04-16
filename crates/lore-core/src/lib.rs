//! # lore-core
//!
//! Shared types, database schema, and async connection management for the
//! Lore documentation server.
//!
//! Every other crate in the workspace depends on this one.  It owns:
//!
//! * **Error type** — [`error::LoreError`], a single enum covering all failure
//!   modes across the workspace.
//! * **Domain types** — [`node`], [`doc`], [`package`], [`search`] modules
//!   exposing the structs that model the data in a Lore `.db` file.
//! * **Database access** — [`db::Db`], an async wrapper around a `SQLite`
//!   database with schema migrations, CRUD operations, and FTS5/vector helpers.

#![deny(clippy::all, clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
// `clippy::pedantic` flags some intentional patterns; suppress selectively.
#![allow(
    clippy::module_name_repetitions, // e.g. `LoreError` in `error` module
    clippy::missing_errors_doc,      // every pub fn returns Result — noted in module docs
    clippy::must_use_candidate       // too aggressive for getters
)]

/// Async database connection, schema management, and CRUD operations.
pub mod db;
/// The [`Doc`] type representing a documentation file.
pub mod doc;
/// The [`LoreError`] type used throughout the workspace.
pub mod error;
/// Floating-point math utilities shared by the build and search pipelines.
pub mod math;
/// [`Node`], [`NewNode`], and [`NodeKind`] — the fundamental unit of indexed content.
pub mod node;
/// [`Package`] and [`PackageMetadata`] — package identity and registry data.
pub mod package;
/// [`SearchConfig`], [`ScoredNode`], and [`SearchResult`] — search pipeline types.
pub mod search;

// Re-export the most commonly used types at the crate root for convenience.
pub use db::Db;
pub use doc::Doc;
pub use error::LoreError;
pub use math::cosine_similarity;
pub use node::{NewNode, Node, NodeKind};
pub use package::{Package, PackageMetadata};
pub use search::{ScoredNode, SearchConfig, SearchResult};
