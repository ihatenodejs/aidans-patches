//! Asset preparation primitives shared by the migration tooling.
//!
//! The icon module is a deliberately narrow, bounded implementation of the
//! existing Python/Pillow icon transform:
//!
//! ```text
//! Image.open(bytes).convert("RGBA").resize((128, 128), LANCZOS)
//! ```
//!
//! followed by PNG encoding. Decoding is supported for the formats the
//! production icon pipeline actually receives (PNG, JPEG and WebP); anything
//! else is rejected explicitly instead of being silently ignored.
//!
//! Pixel arithmetic and resampling follow Pillow 11.3.0 (`src/libImaging`),
//! which is distributed under the MIT-CMU license. See `PILLOW-LICENSE.txt`
//! in this crate for the upstream notice. JPEG decoding uses a pinned
//! libjpeg-turbo 3.1.1 build; see `vendor/turbojpeg-sys/NOTICE.md`.

pub mod icons;

pub use icons::{
    transform_icon, write_icon_if_changed, IconError, IconImage, IconWriteDecision, ICON_SIZE,
};
