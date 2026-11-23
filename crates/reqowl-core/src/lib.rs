//! reqowl core: notebook pages (Markdown files) holding runnable HTTP blocks.
//!
//! The pipeline is: [`page::parse`] finds the ` ```http ` blocks in a page,
//! [`request::parse`] turns one block into a [`request::RequestSpec`],
//! [`vars::Resolver`] fills in `{{variables}}`, and [`exec::execute`] sends it.

pub mod env;
pub mod error;
pub mod exec;
pub mod export;
pub mod history;
pub mod import;
pub mod mask;
pub mod oauth;
pub mod page;
pub mod request;
pub mod session;
pub mod vars;
pub mod ws;

pub use error::{Error, Result};
