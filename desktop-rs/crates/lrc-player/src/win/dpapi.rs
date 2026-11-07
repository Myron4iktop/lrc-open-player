//! Windows' per-user encryption for the saved API keys (`_dpapi` in lrcplayer.py): the
//! same `CryptProtectData` call with UI forbidden and no extra entropy, so keys saved
//! by the Python app read back here, and the other way round.

use lrc_core::ai::Cipher;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;

pub struct Dpapi;

fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).ok()?,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `data`, which outlives the call; Windows allocates
    // `output`, which is copied and then freed with LocalFree as the API asks.
    unsafe {
        let ok = if protect {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        ok.ok()?;
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Some(out)
    }
}

impl Cipher for Dpapi {
    fn protect(&self, data: &[u8]) -> Option<Vec<u8>> {
        run(data, true)
    }

    fn unprotect(&self, data: &[u8]) -> Option<Vec<u8>> {
        run(data, false)
    }
}
