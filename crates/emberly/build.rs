//! Stamps `EMBERLY_BUILD_TIMESTAMP` for `--version` (Cargo's own version
//! string only changes on a release bump, so it can't tell you whether the
//! binary in hand is today's local build or last week's).

fn main() {
    println!(
        "cargo:rustc-env=EMBERLY_BUILD_TIMESTAMP={}",
        build_timestamp()
    );
}

fn build_timestamp() -> String {
    let format = match time::format_description::parse_borrowed::<2>(
        "[year][month][day]-[hour][minute][second]",
    ) {
        Ok(format) => format,
        Err(_) => return "unknown".to_string(),
    };
    time::OffsetDateTime::now_utc()
        .format(&format)
        .unwrap_or_else(|_| "unknown".to_string())
}
