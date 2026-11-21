//! `zend-zgx`, the desktop app's executable: the app itself is the library
//! (src/lib.rs), which Android's entry (src/android.rs) runs too.

#![cfg_attr(all(windows, feature = "beta"), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    zend_zgx::main()
}
