/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

pub mod id;
pub mod stream;
pub mod text;

pub use id::Id;
pub use stream::EntryStream;
pub use text::{Folded, Token};
