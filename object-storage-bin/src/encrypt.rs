use crypto::aes::ecb_decryptor;
use crypto::aes::KeySize;
use crypto::blockmodes::PkcsPadding;
use crypto::buffer::{BufferResult, ReadBuffer, RefReadBuffer, RefWriteBuffer, WriteBuffer};
use log;
use tihu::SharedString;

pub fn decrypt_by_aes_256(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, SharedString> {
    let mut decryptor = ecb_decryptor(KeySize::KeySize256, key, PkcsPadding);
    let mut final_result = Vec::<u8>::new();
    let mut read_buffer = RefReadBuffer::new(data);
    let mut buffer = [0; 4096];
    let mut write_buffer = RefWriteBuffer::new(&mut buffer);
    loop {
        let result = decryptor
            .decrypt(&mut read_buffer, &mut write_buffer, true)
            .map_err(|err| -> SharedString {
                log::error!("Aes decrypt failed: {:?}", err);
                SharedString::from_static("Aes decrypt failed")
            })?;
        final_result.extend(
            write_buffer
                .take_read_buffer()
                .take_remaining()
                .iter()
                .map(|&i| i),
        );
        match result {
            BufferResult::BufferUnderflow => break,
            BufferResult::BufferOverflow => {}
        }
    }
    Ok(final_result)
}
