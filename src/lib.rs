#![allow(clippy::single_match)]
#![allow(clippy::new_without_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::len_without_is_empty)]
#![allow(clippy::explicit_counter_loop)]
#![allow(clippy::while_let_on_iterator)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::let_and_return)]

pub mod base_hash;
pub mod build;
pub mod cache;
pub mod command;
pub mod common;
pub mod config;
pub mod directory;
pub mod hash;
pub mod interner;
pub mod plan;
pub mod scan;
pub mod thread_pool;
pub mod timestamp;
pub mod tracer;
