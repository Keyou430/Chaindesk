use super::failure;
use anyhow::Result;
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData},
    },
    core::PCWSTR,
};

const HEADER: &[u8] = b"CDP\x01";

pub(super) fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
    let output = transform(plaintext, true)?;
    let mut encoded = HEADER.to_vec();
    encoded.extend(output);
    Ok(encoded)
}

pub(super) fn unprotect(encoded: &[u8]) -> Result<Vec<u8>> {
    let payload = encoded.strip_prefix(HEADER).ok_or_else(|| {
        failure(
            "CREDENTIAL_STORE_FAILED",
            "The protected credential format is not supported.",
        )
    })?;
    transform(payload, false)
}

fn transform(input: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let data = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(input.len())
            .map_err(|_| failure("CREDENTIAL_STORE_FAILED", "Protected data is too large."))?,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // The API owns this allocation even if a later conversion fails.
    let mut output = scopeguard::guard(&mut output, |output| {
        if !output.pbData.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            }
        }
    });
    let result = unsafe {
        if encrypt {
            CryptProtectData(
                &data,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut **output,
            )
        } else {
            CryptUnprotectData(&data, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut **output)
        }
    };
    result.map_err(|_| failure("CREDENTIAL_STORE_FAILED", "Windows could not protect or read credentials for this user. Re-enter credentials after moving to another computer or account."))?;
    Ok(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec())
}

#[cfg(test)]
mod tests {
    use super::{protect, unprotect};

    #[test]
    fn dpapi_roundtrip_requires_an_intact_blob_and_version() -> anyhow::Result<()> {
        let secret = "temporary-user:密碼&:\"".as_bytes();
        let encoded = protect(secret)?;
        assert!(!encoded.windows(secret.len()).any(|window| window == secret));
        assert_eq!(unprotect(&encoded)?, secret);
        let mut tampered = encoded;
        if let Some(last) = tampered.last_mut() {
            *last ^= 1;
        }
        assert!(unprotect(&tampered).is_err());
        assert!(unprotect(b"unknown").is_err());
        Ok(())
    }
}
