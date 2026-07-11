//! P-8 guard: Z.ai (and any single vendor) must be reachable by configuration
//! alone — a provider is data, not code. This test fails if a vendor name
//! leaks into the `emberly-providers` source, which would mean a provider had
//! been special-cased in the wire layer instead of expressed as a profile.

use std::path::{Path, PathBuf};

#[test]
fn no_vendor_specific_code_in_providers() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../emberly-providers/src");
    let mut offenders = Vec::new();
    check_dir(&src, &mut offenders);
    assert!(
        offenders.is_empty(),
        "vendor names leaked into emberly-providers (a provider must be config, \
         not code — P-8): {offenders:?}"
    );
}

fn check_dir(dir: &Path, offenders: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("cannot read {}: {e}", dir.display()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            check_dir(&path, offenders);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text.to_ascii_lowercase(),
                Err(e) => panic!("cannot read {}: {e}", path.display()),
            };
            for needle in ["zai", "z.ai", "glm"] {
                if text.contains(needle) {
                    offenders.push(format!("{} contains {needle:?}", path.display()));
                }
            }
        }
    }
}
