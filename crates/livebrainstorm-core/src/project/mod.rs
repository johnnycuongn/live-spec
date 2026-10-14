//! Understanding a project: index it once, brief it once, retrieve per keystroke.
//!
//! Two layers, like a new teammate: the *brief* is the onboarding doc you read
//! once (cached, regenerated when files change); *retrieval* is glancing at the
//! specific files relevant to what you're discussing right now.

pub mod brief;
pub mod digest;
pub mod explore;
pub mod retrieve;
pub mod scan;
pub(crate) mod terms;

pub use brief::{BriefStatus, BriefStore, StoredBrief};
pub use retrieve::{retrieve, shortlist, Snippet};
pub use scan::{scan, FileEntry, ProjectIndex};

/// FNV-1a: tiny, stable across Rust versions (unlike `DefaultHasher`),
/// which matters because we persist these hashes.
#[derive(Clone, Copy)]
pub(crate) struct Fnv(u64);

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}
