use std::fmt::{self, Debug};
use std::ptr::NonNull;

use serde::{Deserialize, Serialize};
use windows::core::{w, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};
use zeroize::{Zeroize, Zeroizing};

const CRED_TARGET_NAME: PCWSTR = w!("power-controle/dell-cctk-bios-password");

fn is_credential_absent(error: &windows::core::Error) -> bool {
    error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiosPasswordKind {
    Setup,
    System,
}

impl BiosPasswordKind {
    const fn stored_name(self) -> PCWSTR {
        match self {
            Self::Setup => w!("Setup"),
            Self::System => w!("System"),
        }
    }

    fn parse_stored_name(value: &str) -> Result<Self, CredentialError> {
        match value {
            "Setup" => Ok(Self::Setup),
            "System" => Ok(Self::System),
            _ => Err(CredentialError::InvalidCredential),
        }
    }
}

pub struct BiosPassword {
    pub kind: BiosPasswordKind,
    pub password: Zeroizing<String>,
}

impl Debug for BiosPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BiosPassword")
            .field("kind", &self.kind)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BiosPasswordStatus {
    pub configured: bool,
    pub kind: Option<BiosPasswordKind>,
}

#[derive(Debug)]
pub enum CredentialError {
    InvalidCredential,
    InvalidPassword,
    Platform,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredential => formatter.write_str("Saved BIOS credential is invalid"),
            Self::InvalidPassword => formatter.write_str("BIOS password cannot be empty"),
            Self::Platform => formatter.write_str("Windows Credential Manager operation failed"),
        }
    }
}

impl std::error::Error for CredentialError {}

struct CredentialAllocation(NonNull<CREDENTIALW>);

impl CredentialAllocation {
    /// # Safety
    /// `pointer` must be the non-null allocation returned by `CredReadW` and must not have
    /// previously been passed to `CredFree`.
    unsafe fn new(pointer: *mut CREDENTIALW) -> Result<Self, CredentialError> {
        NonNull::new(pointer)
            .map(Self)
            .ok_or(CredentialError::InvalidCredential)
    }

    fn credential(&self) -> &CREDENTIALW {
        // SAFETY: `CredentialAllocation` owns a live non-null allocation returned by CredReadW.
        // It is freed only in Drop, after this borrow ends.
        unsafe { self.0.as_ref() }
    }
}

impl Drop for CredentialAllocation {
    fn drop(&mut self) {
        // SAFETY: this allocation came from `CredReadW` and is owned exactly once by this guard.
        unsafe { CredFree(self.0.as_ptr().cast()) };
    }
}

pub fn get_bios_password() -> Result<Option<BiosPassword>, CredentialError> {
    let mut credential_pointer = std::ptr::null_mut();
    // SAFETY: the target name is a static null-terminated UTF-16 string and the output pointer
    // is valid storage for CredReadW to initialize.
    let result = unsafe {
        CredReadW(
            CRED_TARGET_NAME,
            CRED_TYPE_GENERIC,
            None,
            &mut credential_pointer,
        )
    };
    let allocation = match result {
        Ok(()) => {
            // SAFETY: a successful CredReadW initializes credential_pointer with a CredFree-owned allocation.
            unsafe { CredentialAllocation::new(credential_pointer)? }
        }
        Err(error) if is_credential_absent(&error) => return Ok(None),
        Err(_) => return Err(CredentialError::Platform),
    };

    let credential = allocation.credential();
    if credential.UserName.is_null()
        || credential.CredentialBlob.is_null()
        || credential.CredentialBlobSize == 0
    {
        return Err(CredentialError::InvalidCredential);
    }

    // SAFETY: Credential Manager guarantees UserName is a null-terminated UTF-16 string for a
    // successful generic credential read. Its allocation remains owned by `allocation`.
    let kind_name = unsafe { PCWSTR(credential.UserName.0).to_string() }
        .map_err(|_| CredentialError::InvalidCredential)?;
    let kind = BiosPasswordKind::parse_stored_name(&kind_name)?;

    // SAFETY: CredentialBlob points to CredentialBlobSize bytes owned by `allocation`.
    let password_bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    let password = String::from_utf8(password_bytes.to_vec())
        .map_err(|_| CredentialError::InvalidCredential)?;

    Ok(Some(BiosPassword {
        kind,
        password: Zeroizing::new(password),
    }))
}

pub fn set_bios_password(kind: BiosPasswordKind, password: &str) -> Result<(), CredentialError> {
    if password.is_empty() {
        return Err(CredentialError::InvalidPassword);
    }

    let mut password_bytes = Zeroizing::new(password.as_bytes().to_vec());
    let byte_count =
        u32::try_from(password_bytes.len()).map_err(|_| CredentialError::InvalidPassword)?;
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(CRED_TARGET_NAME.0.cast_mut()),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(kind.stored_name().0.cast_mut()),
        CredentialBlobSize: byte_count,
        CredentialBlob: password_bytes.as_mut_ptr(),
        ..Default::default()
    };

    // SAFETY: CREDENTIALW references static target/kind strings and a live password buffer for
    // the duration of this synchronous call. Credential Manager copies the blob before returning.
    unsafe { CredWriteW(&credential, 0) }.map_err(|_| CredentialError::Platform)?;
    password_bytes.zeroize();
    Ok(())
}

pub fn clear_bios_password() -> Result<(), CredentialError> {
    // SAFETY: the target name is a static null-terminated UTF-16 string.
    match unsafe { CredDeleteW(CRED_TARGET_NAME, CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(error) if is_credential_absent(&error) => Ok(()),
        Err(_) => Err(CredentialError::Platform),
    }
}

pub fn get_status() -> Result<BiosPasswordStatus, CredentialError> {
    Ok(match get_bios_password()? {
        Some(password) => BiosPasswordStatus {
            configured: true,
            kind: Some(password.kind),
        },
        None => BiosPasswordStatus {
            configured: false,
            kind: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bios_password_kind_deserializes_from_settings_payload() {
        assert_eq!(
            serde_json::from_str::<BiosPasswordKind>("\"setup\"").ok(),
            Some(BiosPasswordKind::Setup)
        );
        assert_eq!(
            serde_json::from_str::<BiosPasswordKind>("\"system\"").ok(),
            Some(BiosPasswordKind::System)
        );
    }

    #[test]
    fn stored_password_kind_rejects_unrecognized_values() {
        assert!(BiosPasswordKind::parse_stored_name("other").is_err());
    }
}
