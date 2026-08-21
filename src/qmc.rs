//! QMCv2 encrypted audio decryption (QMCDecode Map/segmented RC4 + ekey TEA).
//!
//! The QMC cipher and key derivation are independent Rust adaptations of
//! QMCDecode (MIT). See `THIRD_PARTY_NOTICES.md`.
//!
//! Only decrypt audio that you legally downloaded and are authorized to use.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use zeroize::Zeroizing;

use crate::error::{QmError, Result};

const FIRST_SEGMENT_SIZE: usize = 128;
const SEGMENT_SIZE: usize = 5_120;
const V2_PREFIX: &[u8] = b"QQMusic EncV2,Key:";
const V2_MIX_KEY_ONE: [u8; 16] = [
    0x33, 0x38, 0x36, 0x5a, 0x4a, 0x59, 0x21, 0x40, 0x23, 0x2a, 0x24, 0x25, 0x5e, 0x26, 0x29, 0x28,
];
const V2_MIX_KEY_TWO: [u8; 16] = [
    0x2a, 0x2a, 0x23, 0x21, 0x28, 0x23, 0x24, 0x25, 0x26, 0x5e, 0x61, 0x31, 0x63, 0x5a, 0x2c, 0x54,
];

fn qmc_error(message: impl Into<String>) -> QmError {
    QmError::ApiData(message.into())
}

fn derive_key(raw_key: &str) -> Result<Zeroizing<Vec<u8>>> {
    let raw_key = raw_key.trim();
    if raw_key.is_empty() {
        return Err(qmc_error("EKey cannot be empty"));
    }

    let mut decoded = Zeroizing::new(
        STANDARD
            .decode(raw_key)
            .map_err(|_| qmc_error("EKey is not valid base64"))?,
    );
    if let Some(encrypted_v2) = decoded.strip_prefix(V2_PREFIX) {
        let first_pass = decrypt_tencent_tea(encrypted_v2, &V2_MIX_KEY_ONE)?;
        let second_pass = decrypt_tencent_tea(&first_pass, &V2_MIX_KEY_TWO)?;
        decoded = Zeroizing::new(
            STANDARD
                .decode(&second_pass)
                .map_err(|_| qmc_error("EKey has an invalid EncV2 wrapper"))?,
        );
    }
    if decoded.len() < 24 || (decoded.len() - 8) % 8 != 0 {
        return Err(qmc_error("EKey has an invalid derived-key length"));
    }

    let simple_key = simple_make_key(106);
    let mut tea_key = Zeroizing::new([0_u8; 16]);
    for index in 0..8 {
        tea_key[index * 2] = simple_key[index];
        tea_key[index * 2 + 1] = decoded[index];
    }
    let decrypted = decrypt_tencent_tea(&decoded[8..], &tea_key)?;
    decoded.truncate(8);
    decoded.extend_from_slice(&decrypted);
    Ok(decoded)
}

fn simple_make_key(seed: u8) -> [u8; 8] {
    std::array::from_fn(|index| ((f64::from(seed) + index as f64 * 0.1).tan().abs() * 100.0) as u8)
}

fn decrypt_tencent_tea(input: &[u8], key: &[u8; 16]) -> Result<Zeroizing<Vec<u8>>> {
    const SALT_LENGTH: usize = 2;
    const ZERO_LENGTH: usize = 7;
    if input.len() < 16 || !input.len().is_multiple_of(8) {
        return Err(qmc_error("EKey has an invalid TEA payload"));
    }

    let tea_key = Zeroizing::new([
        u32::from_be_bytes(key[0..4].try_into().expect("four-byte TEA word")),
        u32::from_be_bytes(key[4..8].try_into().expect("four-byte TEA word")),
        u32::from_be_bytes(key[8..12].try_into().expect("four-byte TEA word")),
        u32::from_be_bytes(key[12..16].try_into().expect("four-byte TEA word")),
    ]);
    let mut block = Zeroizing::new(tea_decrypt_block(&input[..8], &tea_key)?);
    let padding = usize::from(block[0] & 0x07);
    if padding + SALT_LENGTH != 8 {
        return Err(qmc_error("EKey has an invalid TEA padding length"));
    }
    let Some(output_length) = input
        .len()
        .checked_sub(1 + padding + SALT_LENGTH + ZERO_LENGTH)
    else {
        return Err(qmc_error("EKey has an invalid TEA payload"));
    };
    let mut output = Zeroizing::new(vec![0_u8; output_length]);
    let mut previous_cipher = Zeroizing::new([0_u8; 8]);
    let mut current_cipher = Zeroizing::new(
        input[..8]
            .try_into()
            .map_err(|_| qmc_error("EKey has an invalid TEA payload"))?,
    );
    let mut input_position = 8_usize;
    let mut block_position = 1 + padding;

    let advance_block = |block: &mut [u8; 8],
                         previous_cipher: &mut [u8; 8],
                         current_cipher: &mut [u8; 8],
                         input_position: &mut usize|
     -> Result<()> {
        let end = input_position
            .checked_add(8)
            .filter(|end| *end <= input.len())
            .ok_or_else(|| qmc_error("EKey has an invalid TEA payload"))?;
        *previous_cipher = *current_cipher;
        *current_cipher = input[*input_position..end]
            .try_into()
            .map_err(|_| qmc_error("EKey has an invalid TEA payload"))?;
        for (byte, cipher) in block.iter_mut().zip(current_cipher.iter()) {
            *byte ^= *cipher;
        }
        *block = tea_decrypt_block(block, &tea_key)?;
        *input_position = end;
        Ok(())
    };

    for _ in 0..SALT_LENGTH {
        if block_position == 8 {
            advance_block(
                &mut block,
                &mut previous_cipher,
                &mut current_cipher,
                &mut input_position,
            )?;
            block_position = 0;
        }
        block_position += 1;
    }

    for output_byte in output.iter_mut() {
        if block_position == 8 {
            advance_block(
                &mut block,
                &mut previous_cipher,
                &mut current_cipher,
                &mut input_position,
            )?;
            block_position = 0;
        }
        *output_byte = block[block_position] ^ previous_cipher[block_position];
        block_position += 1;
    }

    for _ in 0..ZERO_LENGTH {
        if block_position == 8 {
            advance_block(
                &mut block,
                &mut previous_cipher,
                &mut current_cipher,
                &mut input_position,
            )?;
            block_position = 0;
        }
        if block[block_position] != previous_cipher[block_position] {
            return Err(qmc_error("EKey TEA padding check failed"));
        }
        block_position += 1;
    }
    Ok(output)
}

fn tea_decrypt_block(input: &[u8], key: &[u32; 4]) -> Result<[u8; 8]> {
    if input.len() != 8 {
        return Err(qmc_error("EKey has an invalid TEA block"));
    }
    let mut left = u32::from_be_bytes(
        input[..4]
            .try_into()
            .map_err(|_| qmc_error("EKey has an invalid TEA block"))?,
    );
    let mut right = u32::from_be_bytes(
        input[4..]
            .try_into()
            .map_err(|_| qmc_error("EKey has an invalid TEA block"))?,
    );
    let delta = 0x9e37_79b9_u32;
    let mut sum = delta.wrapping_mul(16);
    for _ in 0..16 {
        right = right.wrapping_sub(
            ((left << 4).wrapping_add(key[2]))
                ^ left.wrapping_add(sum)
                ^ ((left >> 5).wrapping_add(key[3])),
        );
        left = left.wrapping_sub(
            ((right << 4).wrapping_add(key[0]))
                ^ right.wrapping_add(sum)
                ^ ((right >> 5).wrapping_add(key[1])),
        );
        sum = sum.wrapping_sub(delta);
    }
    let mut output = [0_u8; 8];
    output[..4].copy_from_slice(&left.to_be_bytes());
    output[4..].copy_from_slice(&right.to_be_bytes());
    Ok(output)
}

/// Decrypt an ekey into its QMCv2 master key.
pub fn ekey_decrypt(ekey: &str) -> Result<Vec<u8>> {
    derive_key(ekey).map(|key| key.to_vec())
}

/// QMCDecode Map cipher for short QMCv2 master keys.
pub struct Qmc2Map {
    key: Zeroizing<Vec<u8>>,
}

impl Qmc2Map {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.is_empty() {
            return Err(qmc_error("Qmc2Map key cannot be empty"));
        }
        Ok(Self {
            key: Zeroizing::new(key.to_vec()),
        })
    }

    pub fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        let mut output = data.to_vec();
        self.decrypt_in_place(&mut output, offset);
        output
    }

    fn decrypt_in_place(&self, data: &mut [u8], offset: usize) {
        for (index, byte) in data.iter_mut().enumerate() {
            let position = offset.saturating_add(index);
            let position = if position > 0x7fff {
                position % 0x7fff
            } else {
                position
            };
            let key_index = position.wrapping_mul(position).wrapping_add(71_214) % self.key.len();
            // QMCDecode uses this non-circular mask, rather than u8::rotate_left.
            let rotate = ((key_index & 7) + 4) % 8;
            let value = self.key[key_index];
            *byte ^= (value << rotate) | (value >> rotate);
        }
    }
}

/// QMCDecode segmented RC4 cipher for long QMCv2 master keys.
pub struct Qmc2Rc4 {
    key: Zeroizing<Vec<u8>>,
    seed_box: Zeroizing<Vec<u8>>,
    hash: u32,
}

impl Qmc2Rc4 {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.is_empty() {
            return Err(qmc_error("Qmc2Rc4 key cannot be empty"));
        }
        let key = Zeroizing::new(key.to_vec());
        let length = key.len();
        let mut seed_box = Zeroizing::new((0..length).map(|index| index as u8).collect::<Vec<_>>());
        let mut right = 0_usize;
        for left in 0..length {
            right = (right + usize::from(seed_box[left]) + usize::from(key[left])) % length;
            seed_box.swap(left, right);
        }
        let mut hash = 1_u32;
        for &value in key.iter() {
            if value == 0 {
                continue;
            }
            let next = hash.wrapping_mul(u32::from(value));
            if next == 0 || next <= hash {
                break;
            }
            hash = next;
        }
        Ok(Self {
            key,
            seed_box,
            hash,
        })
    }

    pub fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        let mut output = data.to_vec();
        self.decrypt_in_place(&mut output, offset);
        output
    }

    fn segment_skip(&self, index: usize) -> usize {
        let seed = usize::from(self.key[index % self.key.len()]);
        if seed == 0 {
            return 0;
        }
        ((f64::from(self.hash) / ((index + 1) * seed) as f64 * 100.0) as usize) % self.key.len()
    }

    fn decrypt_in_place(&self, data: &mut [u8], offset: usize) {
        let mut processed = 0_usize;
        let mut position = offset;
        if position < FIRST_SEGMENT_SIZE {
            let length = data.len().min(FIRST_SEGMENT_SIZE - position);
            self.decrypt_first(&mut data[..length], position);
            processed += length;
            position += length;
        }
        while processed < data.len() {
            let length = (SEGMENT_SIZE - position % SEGMENT_SIZE).min(data.len() - processed);
            self.decrypt_segment(&mut data[processed..processed + length], position);
            processed += length;
            position += length;
        }
    }

    fn decrypt_first(&self, data: &mut [u8], offset: usize) {
        for (index, byte) in data.iter_mut().enumerate() {
            *byte ^= self.key[self.segment_skip(offset + index)];
        }
    }

    fn decrypt_segment(&self, data: &mut [u8], offset: usize) {
        let mut state = self.seed_box.clone();
        let skip = offset % SEGMENT_SIZE + self.segment_skip(offset / SEGMENT_SIZE);
        let mut left = 0_usize;
        let mut right = 0_usize;
        for stream_index in 0..skip.saturating_add(data.len()) {
            left = (left + 1) % state.len();
            right = (right + usize::from(state[left])) % state.len();
            state.swap(left, right);
            if stream_index >= skip {
                let target = stream_index - skip;
                let key_index =
                    (usize::from(state[left]) + usize::from(state[right])) % state.len();
                data[target] ^= state[key_index];
            }
        }
    }
}

enum Qmc2Cipher {
    Map(Qmc2Map),
    Rc4(Qmc2Rc4),
}

impl Qmc2Cipher {
    fn new(key: &[u8]) -> Result<Self> {
        if key.len() > 300 {
            Ok(Self::Rc4(Qmc2Rc4::new(key)?))
        } else {
            Ok(Self::Map(Qmc2Map::new(key)?))
        }
    }

    fn decrypt(&self, data: &[u8], offset: usize) -> Vec<u8> {
        match self {
            Self::Map(cipher) => cipher.decrypt(data, offset),
            Self::Rc4(cipher) => cipher.decrypt(data, offset),
        }
    }
}

fn syncsafe_int(bytes: &[u8]) -> usize {
    if bytes.iter().any(|byte| byte & 0x80 != 0) {
        return 0;
    }
    ((bytes[0] as usize) << 21)
        | ((bytes[1] as usize) << 14)
        | ((bytes[2] as usize) << 7)
        | bytes[3] as usize
}

fn tag_header_size(buffer: &[u8], offset: usize) -> usize {
    if buffer.len() < offset + 10 {
        return 0;
    }
    let data = &buffer[offset..];
    if data.starts_with(b"TAG") {
        return 128;
    }
    if data.starts_with(b"ID3") {
        return 10 + syncsafe_int(&data[6..10]);
    }
    if data.starts_with(b"APETAGEX") {
        if data.len() < 32 {
            return 0;
        }
        let extra = u32::from_le_bytes(data[0x0c..0x10].try_into().expect("four-byte APE size"));
        return 32 + extra as usize;
    }
    0
}

/// Detect the decrypted audio extension (`bin` means unknown).
pub fn detect_audio_type(data: &[u8]) -> String {
    let mut offset = 0_usize;
    for _ in 0..5 {
        let length = tag_header_size(data, offset);
        if length == 0 {
            break;
        }
        offset = offset.saturating_add(length);
    }
    if data.len() < offset.saturating_add(0x10) {
        return "bin".into();
    }
    let buffer = &data[offset..];
    let magic4 = &buffer[..4];
    let extension = match magic4 {
        b"fLaC" => "flac",
        b"OggS" => "ogg",
        b"FRM8" => "dff",
        b"RIFF" => "wav",
        b"MAC " => "ape",
        _ => "",
    };
    if !extension.is_empty() {
        return extension.into();
    }
    let magic = u32::from_be_bytes(magic4.try_into().expect("four-byte audio magic"));
    if (magic & 0xfff6_0000) == 0xfff0_0000 {
        return "aac".into();
    }
    if buffer.len() >= 12 && &buffer[4..8] == b"ftyp" {
        let major = &buffer[8..12];
        if major == b"isom" || major == b"iso2" || major == b"MSNV" {
            return "mp4".into();
        }
        if major == b"NDAS" {
            return "m4a".into();
        }
        let major3 = &buffer[8..11];
        return match major3 {
            b"M4A" => "m4a".into(),
            b"M4B" => "m4b".into(),
            b"mp4" => "mp4".into(),
            _ => "bin".into(),
        };
    }
    "bin".into()
}

/// Detect an extension from the beginning of decrypted audio data.
pub fn detect_audio_extension(data: &[u8]) -> String {
    detect_audio_type(&data[..0x100.min(data.len())])
}

/// Decrypt QMCv2 bytes with an ekey and return the audio bytes and extension.
///
/// QMCDecode does not parse legacy file footers or provide the QMCv1 static
/// key, so `ekey_override` must be `Some` and the input must contain only the
/// encrypted media bytes.
pub fn decrypt_qmc(data: &[u8], ekey_override: Option<&str>) -> Result<(Vec<u8>, String)> {
    let ekey = ekey_override
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| qmc_error("QMCv2 decryption requires an ekey"))?;
    let master_key = derive_key(ekey)?;
    let cipher = Qmc2Cipher::new(&master_key)?;
    let output = cipher.decrypt(data, 0);
    let extension = detect_audio_extension(&output);
    Ok((output, extension))
}

/// Read and decrypt a QMCv2 file. An ekey is required.
pub fn decrypt_file(
    input_path: &std::path::Path,
    ekey_override: Option<&str>,
) -> Result<(Vec<u8>, String)> {
    let data = std::fs::read(input_path).map_err(|error| QmError::Io(error.to_string()))?;
    decrypt_qmc(&data, ekey_override)
}

/// Decrypt a QMCv2 file into `output_dir`, using the detected extension.
pub fn decrypt_file_to(
    input_path: &std::path::Path,
    output_dir: &std::path::Path,
    ekey_override: Option<&str>,
) -> Result<std::path::PathBuf> {
    let (output, extension) = decrypt_file(input_path, ekey_override)?;
    std::fs::create_dir_all(output_dir).map_err(|error| QmError::Io(error.to_string()))?;
    let stem = input_path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".into());
    let output_path = output_dir.join(format!("{stem}.{extension}"));
    std::fs::write(&output_path, output).map_err(|error| QmError::Io(error.to_string()))?;
    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tea_encrypt_block(input: &[u8; 8], key: &[u8; 16]) -> [u8; 8] {
        let key = [
            u32::from_be_bytes(key[0..4].try_into().expect("key word")),
            u32::from_be_bytes(key[4..8].try_into().expect("key word")),
            u32::from_be_bytes(key[8..12].try_into().expect("key word")),
            u32::from_be_bytes(key[12..16].try_into().expect("key word")),
        ];
        let mut left = u32::from_be_bytes(input[..4].try_into().expect("left word"));
        let mut right = u32::from_be_bytes(input[4..].try_into().expect("right word"));
        let delta = 0x9e37_79b9_u32;
        let mut sum = 0_u32;
        for _ in 0..16 {
            sum = sum.wrapping_add(delta);
            left = left.wrapping_add(
                ((right << 4).wrapping_add(key[0]))
                    ^ right.wrapping_add(sum)
                    ^ ((right >> 5).wrapping_add(key[1])),
            );
            right = right.wrapping_add(
                ((left << 4).wrapping_add(key[2]))
                    ^ left.wrapping_add(sum)
                    ^ ((left >> 5).wrapping_add(key[3])),
            );
        }
        let mut output = [0_u8; 8];
        output[..4].copy_from_slice(&left.to_be_bytes());
        output[4..].copy_from_slice(&right.to_be_bytes());
        output
    }

    fn encrypt_tencent_tea_for_test(body: &[u8], key: &[u8; 16]) -> Vec<u8> {
        let padding = (8 - (body.len() + 10) % 8) % 8;
        let mut plain = Vec::with_capacity(body.len() + padding + 10);
        plain.push(padding as u8);
        plain.extend((0..padding).map(|index| 0xa0_u8.wrapping_add(index as u8)));
        plain.extend_from_slice(&[0x5a, 0xa5]);
        plain.extend_from_slice(body);
        plain.extend_from_slice(&[0_u8; 7]);
        assert!(plain.len().is_multiple_of(8));

        let mut previous_plain = [0_u8; 8];
        let mut previous_cipher = [0_u8; 8];
        let mut output = Vec::with_capacity(plain.len());
        for chunk in plain.chunks_exact(8) {
            let mut mixed = [0_u8; 8];
            for index in 0..8 {
                mixed[index] = chunk[index] ^ previous_cipher[index];
            }
            let encrypted = tea_encrypt_block(&mixed, key);
            let mut cipher = [0_u8; 8];
            for index in 0..8 {
                cipher[index] = encrypted[index] ^ previous_plain[index];
            }
            output.extend_from_slice(&cipher);
            previous_plain = mixed;
            previous_cipher = cipher;
        }
        output
    }

    fn encode_test_ekey(clear_key: &[u8], version_two: bool) -> String {
        assert!(clear_key.len() >= 16);
        let simple_key = simple_make_key(106);
        let mut tea_key = [0_u8; 16];
        for index in 0..8 {
            tea_key[index * 2] = simple_key[index];
            tea_key[index * 2 + 1] = clear_key[index];
        }
        let mut version_one = clear_key[..8].to_vec();
        version_one.extend(encrypt_tencent_tea_for_test(&clear_key[8..], &tea_key));
        if !version_two {
            return STANDARD.encode(version_one);
        }
        let encoded_v1 = STANDARD.encode(version_one);
        let inner = encrypt_tencent_tea_for_test(encoded_v1.as_bytes(), &V2_MIX_KEY_TWO);
        let outer = encrypt_tencent_tea_for_test(&inner, &V2_MIX_KEY_ONE);
        let mut wrapped = V2_PREFIX.to_vec();
        wrapped.extend(outer);
        STANDARD.encode(wrapped)
    }

    fn assert_chunk_boundary_independent(cipher: &Qmc2Cipher) {
        let original = (0..25_123).map(|value| value as u8).collect::<Vec<_>>();
        let mut encrypted = cipher.decrypt(&original, 0);
        assert_ne!(encrypted, original);
        let mut offset = 0_usize;
        for chunk in encrypted.chunks_mut(777) {
            match cipher {
                Qmc2Cipher::Map(map) => map.decrypt_in_place(chunk, offset),
                Qmc2Cipher::Rc4(rc4) => rc4.decrypt_in_place(chunk, offset),
            }
            offset += chunk.len();
        }
        assert_eq!(encrypted, original);
    }

    #[test]
    fn map_and_segmented_rc4_are_chunk_boundary_independent() {
        assert_chunk_boundary_independent(
            &Qmc2Cipher::new(&(1..=128).collect::<Vec<_>>()).expect("map key"),
        );
        assert_chunk_boundary_independent(
            &Qmc2Cipher::new(
                &(0..512)
                    .map(|value| (value % 251 + 1) as u8)
                    .collect::<Vec<_>>(),
            )
            .expect("rc4 key"),
        );
    }

    #[test]
    fn version_one_and_version_two_ekeys_derive_the_same_key() {
        let clear_key = (0..512)
            .map(|value| (value % 251 + 1) as u8)
            .collect::<Vec<_>>();
        for version_two in [false, true] {
            let encoded = encode_test_ekey(&clear_key, version_two);
            assert_eq!(ekey_decrypt(&encoded).expect("derive key"), clear_key);
        }
    }

    #[test]
    fn invalid_ekeys_are_rejected() {
        assert!(ekey_decrypt("").is_err());
        assert!(ekey_decrypt("not-base64").is_err());
        let invalid_v2 = STANDARD.encode([V2_PREFIX, b"too short"].concat());
        assert!(ekey_decrypt(&invalid_v2).is_err());
    }

    #[test]
    fn unexpected_tea_padding_length_is_rejected() {
        let payload = encrypt_tencent_tea_for_test(b"x", &V2_MIX_KEY_ONE);
        let error =
            decrypt_tencent_tea(&payload, &V2_MIX_KEY_ONE).expect_err("unexpected padding length");
        assert!(error.to_string().contains("padding length"));
    }

    #[test]
    fn decrypt_qmc_requires_an_ekey() {
        let error = decrypt_qmc(b"encrypted", None).expect_err("missing ekey");
        assert!(error.to_string().contains("requires an ekey"));
        assert!(decrypt_qmc(b"encrypted", Some("  ")).is_err());
    }

    #[test]
    fn decrypt_qmc_round_trips_map_and_rc4() {
        for clear_key in [
            (1..=128).collect::<Vec<_>>(),
            (0..512)
                .map(|value| (value % 251 + 1) as u8)
                .collect::<Vec<_>>(),
        ] {
            let mut original = b"fLaC".to_vec();
            original.extend((0..25_119).map(|value| value as u8));
            let cipher = Qmc2Cipher::new(&clear_key).expect("cipher");
            let encrypted = cipher.decrypt(&original, 0);
            let ekey = encode_test_ekey(&clear_key, true);
            let (decrypted, extension) =
                decrypt_qmc(&encrypted, Some(&ekey)).expect("decrypt QMCv2");
            assert_eq!(decrypted, original);
            assert_eq!(extension, "flac");
        }
    }

    #[test]
    fn detects_audio_magic_after_tags() {
        let flac = [b"fLaC".to_vec(), vec![0_u8; 100]].concat();
        assert_eq!(detect_audio_extension(&flac), "flac");
        let ogg = [b"OggS".to_vec(), vec![0_u8; 100]].concat();
        assert_eq!(detect_audio_extension(&ogg), "ogg");

        let mut id3 = b"ID3\x04\x00\x00\x00\x00\x00\x0b".to_vec();
        id3.extend(vec![0_u8; 11]);
        id3.extend_from_slice(b"fLaC");
        id3.extend(vec![0_u8; 100]);
        assert_eq!(detect_audio_extension(&id3), "flac");
    }

    #[test]
    fn empty_cipher_keys_are_rejected() {
        assert!(Qmc2Map::new(&[]).is_err());
        assert!(Qmc2Rc4::new(&[]).is_err());
    }
}
