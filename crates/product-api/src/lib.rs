//! The product: one engine, one local API, and the client that talks to it.
//!
//! # Everything goes through one door
//!
//! The desktop app, the CLI, and the MCP bridge are all clients of the same
//! method table in [`api`]. There is no second path with slightly different
//! rules, so a permission check cannot be present on one route and missing on
//! another, and a capability the interface can run is exactly the set an agent
//! can run.
//!
//! ```text
//!  desktop app ─┐
//!  CLI ─────────┼─→ loopback HTTP ─→ Api::call ─→ Engine
//!  MCP bridge ──┤
//!  agent hooks ─┘
//! ```
//!
//! # Two ways in, on purpose
//!
//! An AI reports what it did by calling a tool, which it does when it thinks
//! of it. A [`hook`] fires on every tool call whether anybody thought about it
//! or not. The first gives depth — an agent can say what it verified — and the
//! second gives breadth, which is what makes this a product that works while
//! you are not thinking about it rather than a demonstration.
//!
//! # The transport is boring on purpose
//!
//! Loopback HTTP with a token in a file only this user can read. No async
//! runtime, no framework, a few hundred lines of thread-per-connection. The
//! whole product idles at a few observations a second; anything more elaborate
//! would be paid for in the one number this project promises to keep small.

#![deny(missing_docs)]

pub mod api;
pub mod client;
pub mod endpoint;
pub mod engine;
pub mod hook;
pub mod http;
pub mod setup;
pub mod view;

pub use api::Api;
pub use client::Client;
pub use endpoint::Endpoint;
pub use engine::Engine;
