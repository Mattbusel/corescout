//! CoreScout as a tool any AI can use.
//!
//! # Why the Model Context Protocol, and only that
//!
//! One integration that many clients already speak beats several that each
//! speak to one. Claude Code, Codex, OpenCode, Cursor and anything else that
//! implements MCP get the same server, the same tools, and the same
//! behaviour, and a client written next year needs nothing from CoreScout.
//!
//! # How the AI comes to know CoreScout is there
//!
//! The initialise handshake carries an `instructions` field, generated from
//! what CoreScout currently knows and what its autonomy mode currently
//! permits. A client puts that in front of the model. Nobody has to write
//! CoreScout into a prompt, and the description cannot drift out of date,
//! because it is produced at connection time from the live state.
//!
//! ```text
//! agent ── initialize ──▶ instructions: what CoreScout is, and what it knows
//!       ── tools/list ──▶ fourteen tools
//!       ── tools/call ──▶ the product API ──▶ the engine
//! ```

#![deny(missing_docs)]

pub mod server;
pub mod tools;

pub use server::{Backend, Server, PROTOCOL_VERSION};
pub use tools::{Tool, TOOLS};
