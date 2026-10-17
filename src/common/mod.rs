// Platform-independent pieces shared by the Windows and Linux front ends.
// Not every piece is used on Linux yet (note animations, website skipping).
#[cfg_attr(not(windows), allow(dead_code))]
pub mod anim;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod settings;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod sites;
