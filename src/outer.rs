pub(crate) const MID_PACKET_SIZE: usize = 296;
pub(crate) const OUTER_PACKET_SIZE: usize = MID_PACKET_SIZE + 16 + 24;

pub(crate) fn encrypt_outer_packet<T, R>(key: &[u8], body: &[u8], callback: T) -> R
where
    T: FnOnce(&mut [u8]) -> R,
{
    assert!(key.len() == 32);
    assert!(body.len() == MID_PACKET_SIZE);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_KEYBYTES == 32);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_NPUBBYTES == 24);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_ABYTES == 16);
    unsafe { assert!(::libsodium_sys::sodium_init() >= 0) };

    let mut result = [0u8; OUTER_PACKET_SIZE];
    let mut output_amt: ::std::os::raw::c_ulonglong = 0;
    let (nonce, tail) = result.split_at_mut(24);
    unsafe {
        ::libsodium_sys::randombytes_buf(
            nonce.as_mut_ptr() as *mut ::core::ffi::c_void,
            nonce.len(),
        );
        ::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_encrypt(
            tail.as_mut_ptr(),
            &mut output_amt as *mut ::std::os::raw::c_ulonglong,
            body.as_ptr(),
            ::num::NumCast::from(body.len()).unwrap(),
            ::std::ptr::null(),
            0,
            ::std::ptr::null(),
            nonce.as_ptr(),
            key.as_ptr(),
        );
    }
    assert!(output_amt == ::num::NumCast::from(tail.len()).unwrap());
    (callback)(&mut result)
}

pub(crate) fn decrypt_outer_packet<T, R>(key: &[u8], packet: &[u8], callback: T) -> Result<R, ()>
where
    T: FnOnce(&mut [u8]) -> R,
{
    assert!(key.len() == 32);
    assert!(packet.len() == OUTER_PACKET_SIZE);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_KEYBYTES == 32);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_NPUBBYTES == 24);
    assert!(::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_ABYTES == 16);
    unsafe { assert!(::libsodium_sys::sodium_init() >= 0) };

    let mut result = [0u8; MID_PACKET_SIZE];
    let mut output_bytes: ::std::os::raw::c_ulonglong = 0;
    let (nonce, payload) = packet.split_at(24);
    let status = unsafe {
        ::libsodium_sys::crypto_aead_xchacha20poly1305_ietf_decrypt(
            result.as_mut_ptr(),
            &mut output_bytes as *mut ::std::os::raw::c_ulonglong,
            ::std::ptr::null::<u8>() as *mut u8,
            payload.as_ptr(),
            ::num::NumCast::from(payload.len()).unwrap(),
            ::std::ptr::null(),
            0,
            nonce.as_ptr(),
            key.as_ptr(),
        )
    };
    if status != 0 {
        return Err(());
    }
    assert!(output_bytes == ::num::NumCast::from(result.len()).unwrap());
    Ok((callback)(&mut result))
}
