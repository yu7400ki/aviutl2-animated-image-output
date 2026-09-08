//! crate の内側からしか見られないものを使う検証

mod basis;

#[path = "../../tests/support/mod.rs"]
mod support;

use crate::{ColorType, Encoder};
use support::*;
