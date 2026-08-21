//! QRC 解密模块中的 Triple-DES 自定义实现.
//!
//! 该模块包含了一个与标准 Triple-DES 密钥扩展略有不同的自定义加密/解密实现,
//! 用于兼容 QQ 音乐 QRC 歌词解密中所使用的特定 3DES 变体。
//! 独立实现，按协议行为编写，未直接使用
//! <https://github.com/L-1124/QQMusicApi> 的 `algorithms/tripledes.py` 源码.

pub const ENCRYPT: i32 = 1;
pub const DECRYPT: i32 = 0;

const SBOX: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10,
        10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

#[inline]
fn sbox_bit(a: u32) -> u32 {
    (a & 32) | ((a & 31) >> 1) | ((a & 1) << 4)
}

/// 初始置换.
fn initial_permutation(input: &[u8]) -> (u32, u32) {
    let v0 = (input[0] as u32)
        | ((input[1] as u32) << 8)
        | ((input[2] as u32) << 16)
        | ((input[3] as u32) << 24);
    let v1 = (input[4] as u32)
        | ((input[5] as u32) << 8)
        | ((input[6] as u32) << 16)
        | ((input[7] as u32) << 24);

    let bit = |v: u32, b: u32| (v >> b) & 1;
    let s0 = (bit(v1, 6) << 31)
        | (bit(v1, 14) << 30)
        | (bit(v1, 22) << 29)
        | (bit(v1, 30) << 28)
        | (bit(v0, 6) << 27)
        | (bit(v0, 14) << 26)
        | (bit(v0, 22) << 25)
        | (bit(v0, 30) << 24)
        | (bit(v1, 4) << 23)
        | (bit(v1, 12) << 22)
        | (bit(v1, 20) << 21)
        | (bit(v1, 28) << 20)
        | (bit(v0, 4) << 19)
        | (bit(v0, 12) << 18)
        | (bit(v0, 20) << 17)
        | (bit(v0, 28) << 16)
        | (bit(v1, 2) << 15)
        | (bit(v1, 10) << 14)
        | (bit(v1, 18) << 13)
        | (bit(v1, 26) << 12)
        | (bit(v0, 2) << 11)
        | (bit(v0, 10) << 10)
        | (bit(v0, 18) << 9)
        | (bit(v0, 26) << 8)
        | (bit(v1, 0) << 7)
        | (bit(v1, 8) << 6)
        | (bit(v1, 16) << 5)
        | (bit(v1, 24) << 4)
        | (bit(v0, 0) << 3)
        | (bit(v0, 8) << 2)
        | (bit(v0, 16) << 1)
        | bit(v0, 24);
    let s1 = (bit(v1, 7) << 31)
        | (bit(v1, 15) << 30)
        | (bit(v1, 23) << 29)
        | (bit(v1, 31) << 28)
        | (bit(v0, 7) << 27)
        | (bit(v0, 15) << 26)
        | (bit(v0, 23) << 25)
        | (bit(v0, 31) << 24)
        | (bit(v1, 5) << 23)
        | (bit(v1, 13) << 22)
        | (bit(v1, 21) << 21)
        | (bit(v1, 29) << 20)
        | (bit(v0, 5) << 19)
        | (bit(v0, 13) << 18)
        | (bit(v0, 21) << 17)
        | (bit(v0, 29) << 16)
        | (bit(v1, 3) << 15)
        | (bit(v1, 11) << 14)
        | (bit(v1, 19) << 13)
        | (bit(v1, 27) << 12)
        | (bit(v0, 3) << 11)
        | (bit(v0, 11) << 10)
        | (bit(v0, 19) << 9)
        | (bit(v0, 27) << 8)
        | (bit(v1, 1) << 7)
        | (bit(v1, 9) << 6)
        | (bit(v1, 17) << 5)
        | (bit(v1, 25) << 4)
        | (bit(v0, 1) << 3)
        | (bit(v0, 9) << 2)
        | (bit(v0, 17) << 1)
        | bit(v0, 25);
    (s0, s1)
}

/// 逆初始置换.
fn inverse_permutation(s0: u32, s1: u32) -> [u8; 8] {
    let bit = |v: u32, b: u32| ((v >> b) & 1) as u8;
    let mut data = [0u8; 8];
    data[3] = (bit(s1, 24) << 7)
        | (bit(s0, 24) << 6)
        | (bit(s1, 16) << 5)
        | (bit(s0, 16) << 4)
        | (bit(s1, 8) << 3)
        | (bit(s0, 8) << 2)
        | (bit(s1, 0) << 1)
        | bit(s0, 0);
    data[2] = (bit(s1, 25) << 7)
        | (bit(s0, 25) << 6)
        | (bit(s1, 17) << 5)
        | (bit(s0, 17) << 4)
        | (bit(s1, 9) << 3)
        | (bit(s0, 9) << 2)
        | (bit(s1, 1) << 1)
        | bit(s0, 1);
    data[1] = (bit(s1, 26) << 7)
        | (bit(s0, 26) << 6)
        | (bit(s1, 18) << 5)
        | (bit(s0, 18) << 4)
        | (bit(s1, 10) << 3)
        | (bit(s0, 10) << 2)
        | (bit(s1, 2) << 1)
        | bit(s0, 2);
    data[0] = (bit(s1, 27) << 7)
        | (bit(s0, 27) << 6)
        | (bit(s1, 19) << 5)
        | (bit(s0, 19) << 4)
        | (bit(s1, 11) << 3)
        | (bit(s0, 11) << 2)
        | (bit(s1, 3) << 1)
        | bit(s0, 3);
    data[7] = (bit(s1, 28) << 7)
        | (bit(s0, 28) << 6)
        | (bit(s1, 20) << 5)
        | (bit(s0, 20) << 4)
        | (bit(s1, 12) << 3)
        | (bit(s0, 12) << 2)
        | (bit(s1, 4) << 1)
        | bit(s0, 4);
    data[6] = (bit(s1, 29) << 7)
        | (bit(s0, 29) << 6)
        | (bit(s1, 21) << 5)
        | (bit(s0, 21) << 4)
        | (bit(s1, 13) << 3)
        | (bit(s0, 13) << 2)
        | (bit(s1, 5) << 1)
        | bit(s0, 5);
    data[5] = (bit(s1, 30) << 7)
        | (bit(s0, 30) << 6)
        | (bit(s1, 22) << 5)
        | (bit(s0, 22) << 4)
        | (bit(s1, 14) << 3)
        | (bit(s0, 14) << 2)
        | (bit(s1, 6) << 1)
        | bit(s0, 6);
    data[4] = (bit(s1, 31) << 7)
        | (bit(s0, 31) << 6)
        | (bit(s1, 23) << 5)
        | (bit(s0, 23) << 4)
        | (bit(s1, 15) << 3)
        | (bit(s0, 15) << 2)
        | (bit(s1, 7) << 1)
        | bit(s0, 7);
    data
}

/// Triple-DES F 函数.
fn f(state: u32, key: &[u8; 6]) -> u32 {
    let t1 = ((state & 1) << 31)
        | ((state & 0xF8000000) >> 1)
        | ((state & 0x1F800000) >> 3)
        | ((state & 0x01F80000) >> 5)
        | ((state & 0x001F8000) >> 7);
    let t2 = ((state & 0x0001F800) << 15)
        | ((state & 0x00001F80) << 13)
        | ((state & 0x000001F8) << 11)
        | ((state & 0x0000001F) << 9)
        | ((state & 0x80000000) >> 23);

    let k0 = ((t1 >> 24) & 0xFF) as u8 ^ key[0];
    let k1 = ((t1 >> 16) & 0xFF) as u8 ^ key[1];
    let k2 = ((t1 >> 8) & 0xFF) as u8 ^ key[2];
    let k3 = ((t2 >> 24) & 0xFF) as u8 ^ key[3];
    let k4 = ((t2 >> 16) & 0xFF) as u8 ^ key[4];
    let k5 = ((t2 >> 8) & 0xFF) as u8 ^ key[5];

    let state = (SBOX[0][sbox_bit((k0 >> 2) as u32) as usize] as u32) << 28
        | (SBOX[1][sbox_bit((((k0 & 0x03) << 4) | (k1 >> 4)) as u32) as usize] as u32) << 24
        | (SBOX[2][sbox_bit((((k1 & 0x0F) << 2) | (k2 >> 6)) as u32) as usize] as u32) << 20
        | (SBOX[3][sbox_bit((k2 & 0x3F) as u32) as usize] as u32) << 16
        | (SBOX[4][sbox_bit((k3 >> 2) as u32) as usize] as u32) << 12
        | (SBOX[5][sbox_bit((((k3 & 0x03) << 4) | (k4 >> 4)) as u32) as usize] as u32) << 8
        | (SBOX[6][sbox_bit((((k4 & 0x0F) << 2) | (k5 >> 6)) as u32) as usize] as u32) << 4
        | SBOX[7][sbox_bit((k5 & 0x3F) as u32) as usize] as u32;

    let bit = |v: u32, b: u32| (v >> b) & 1;
    (bit(state, 16) << 31)
        | (bit(state, 25) << 30)
        | (bit(state, 12) << 29)
        | (bit(state, 11) << 28)
        | (bit(state, 3) << 27)
        | (bit(state, 20) << 26)
        | (bit(state, 4) << 25)
        | (bit(state, 15) << 24)
        | (bit(state, 31) << 23)
        | (bit(state, 17) << 22)
        | (bit(state, 9) << 21)
        | (bit(state, 6) << 20)
        | (bit(state, 27) << 19)
        | (bit(state, 14) << 18)
        | (bit(state, 1) << 17)
        | (bit(state, 22) << 16)
        | (bit(state, 30) << 15)
        | (bit(state, 24) << 14)
        | (bit(state, 8) << 13)
        | (bit(state, 18) << 12)
        | (bit(state, 0) << 11)
        | (bit(state, 5) << 10)
        | (bit(state, 29) << 9)
        | (bit(state, 23) << 8)
        | (bit(state, 13) << 7)
        | (bit(state, 19) << 6)
        | (bit(state, 2) << 5)
        | (bit(state, 26) << 4)
        | (bit(state, 10) << 3)
        | (bit(state, 21) << 2)
        | (bit(state, 28) << 1)
        | bit(state, 7)
}

/// DES 加密/解密块操作.
fn crypt(input: &[u8], key: &[[u8; 6]; 16]) -> [u8; 8] {
    let (mut s0, mut s1) = initial_permutation(input);
    for k in key[..15].iter() {
        let previous_s1 = s1;
        s1 = f(s1, k) ^ s0;
        s0 = previous_s1;
    }
    s0 ^= f(s1, &key[15]);
    inverse_permutation(s0, s1)
}

const KEY_RND_SHIFT: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const KEY_PERM_C: [u32; 28] = [
    56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59,
    51, 43, 35,
];
const KEY_PERM_D: [u32; 28] = [
    62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12, 4,
    27, 19, 11, 3,
];
const KEY_COMPRESSION: [u32; 48] = [
    13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40, 51,
    30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
];

/// DES 密钥扩展算法 (包含自定义的 PC-2 偏移量 Bug).
fn key_schedule(key: &[u8], mode: i32) -> [[u8; 6]; 16] {
    let mut schedule = [[0u8; 6]; 16];
    let v0 = (key[0] as u32)
        | ((key[1] as u32) << 8)
        | ((key[2] as u32) << 16)
        | ((key[3] as u32) << 24);
    let v1 = (key[4] as u32)
        | ((key[5] as u32) << 8)
        | ((key[6] as u32) << 16)
        | ((key[7] as u32) << 24);

    let mut c = 0u32;
    for (i, &b) in KEY_PERM_C.iter().enumerate() {
        let bit = if b < 32 {
            (v0 >> (31 - b)) & 1
        } else {
            (v1 >> (63 - b)) & 1
        };
        c |= bit << (31 - i as u32);
    }
    let mut d = 0u32;
    for (i, &b) in KEY_PERM_D.iter().enumerate() {
        let bit = if b < 32 {
            (v0 >> (31 - b)) & 1
        } else {
            (v1 >> (63 - b)) & 1
        };
        d |= bit << (31 - i as u32);
    }

    for (i, shift) in KEY_RND_SHIFT.iter().copied().enumerate() {
        c = ((c << shift) | (c >> (28 - shift))) & 0xFFFFFFF0;
        d = ((d << shift) | (d >> (28 - shift))) & 0xFFFFFFF0;

        let togen = if mode == DECRYPT { 15 - i } else { i };

        for j in 0..24 {
            let bit = (c >> (31 - KEY_COMPRESSION[j])) & 1;
            schedule[togen][j / 8] |= (bit as u8) << (7 - (j % 8) as u8);
        }
        for j in 24..48 {
            let bit = (d >> (31 - (KEY_COMPRESSION[j] - 27))) & 1;
            schedule[togen][j / 8] |= (bit as u8) << (7 - (j % 8) as u8);
        }
    }
    schedule
}

/// TripleDES 密钥设置 (根据加密/解密模式分发各子密钥).
pub fn tripledes_key_setup(key: &[u8], mode: i32) -> Vec<[[u8; 6]; 16]> {
    if mode == ENCRYPT {
        vec![
            key_schedule(&key[0..8], ENCRYPT),
            key_schedule(&key[8..16], DECRYPT),
            key_schedule(&key[16..24], ENCRYPT),
        ]
    } else {
        vec![
            key_schedule(&key[16..24], DECRYPT),
            key_schedule(&key[8..16], ENCRYPT),
            key_schedule(&key[0..8], DECRYPT),
        ]
    }
}

/// TripleDES 加密/解密算法.
pub fn tripledes_crypt(data: &[u8], key: &[[[u8; 6]; 16]]) -> [u8; 8] {
    let mut out = [0u8; 8];
    out.copy_from_slice(data);
    for round in key {
        out = crypt(&out, round);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        // 标准 3DES-EDE 测试向量.
        let key: &[u8] = b"!@#)(*$%123ZXC!@!@#)(NHL";
        let schedule = tripledes_key_setup(key, ENCRYPT);
        let plain = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let enc = tripledes_crypt(&plain, &schedule);
        let dec_schedule = tripledes_key_setup(key, DECRYPT);
        let dec = tripledes_crypt(&enc, &dec_schedule);
        assert_eq!(dec, plain);
    }
}
