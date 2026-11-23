//! Stamps the build time into the DLL (`HF_ALLY_BUILD`, UTC `yyyymmdd-hhmmss`) so two builds of the same
//! version can be told apart in a tester's log.
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    const SECONDS_PER_DAY: u64 = 86_400;
    const DAYS_PER_ERA: i64 = 146_097;

    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let unix_days = (seconds / SECONDS_PER_DAY) as i64;
    let seconds_today = seconds % SECONDS_PER_DAY;

    // Howard Hinnant's civil-from-days algorithm uses 400-year Gregorian eras
    // and counts months from March to keep the leap day at the end of the year.
    let shifted_days = unix_days + 719_468;
    let era = shifted_days.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted_days.rem_euclid(DAYS_PER_ERA);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    println!(
        "cargo:rustc-env=HF_ALLY_BUILD={year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        seconds_today / 3_600,
        seconds_today % 3_600 / 60,
        seconds_today % 60
    );
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
}
