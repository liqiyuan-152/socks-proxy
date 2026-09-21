#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use socks_proxy::{
        platform::windows::{CredentialError, DpapiCredentialVault},
        storage::CredentialVault,
    };
    use std::{env, fs};

    let directory = env::args()
        .nth(1)
        .ok_or("usage: dpapi_smoke.exe <test-directory>")?;
    let _ = fs::remove_dir_all(&directory);
    let mut vault = DpapiCredentialVault::new(&directory);
    let secret = b"controlled-rust-dpapi-value";
    let reference = vault.create_version(secret)?;
    let path = std::path::Path::new(&directory).join(format!("{}.cred", reference.as_str()));
    let ciphertext = fs::read(&path)?;
    let contains_plaintext = ciphertext
        .windows(secret.len())
        .any(|window| window == secret);
    let round_trip = vault.read_version(&reference)?.expose() == secret;
    vault.delete_version(&reference)?;
    let deleted = !path.exists();
    let missing_reported = matches!(
        vault.read_version(&reference),
        Err(CredentialError::Missing)
    );
    let _ = fs::remove_dir_all(&directory);

    println!(
        "{{\"passed\":{},\"roundTrip\":{},\"containsPlaintext\":{},\"deleted\":{},\"missingReported\":{}}}",
        round_trip && !contains_plaintext && deleted && missing_reported,
        round_trip,
        contains_plaintext,
        deleted,
        missing_reported
    );
    if round_trip && !contains_plaintext && deleted && missing_reported {
        Ok(())
    } else {
        Err("DPAPI smoke assertions failed".into())
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This example runs only on Windows.");
    std::process::exit(2);
}
