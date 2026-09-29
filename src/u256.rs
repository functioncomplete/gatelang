//! 极简 256 位无符号整数（零依赖），供词级重写层表示 `mod 2ʷ` 的系数与常量。
//!
//! 只实现词级层需要的运算：`+ - *`（均 `mod 2^256`）、按位 `& | ^`、掩码、
//! 比较、与 `u128` 互转。乘法**只保留低 256 位**（高位丢弃）—— 这正是
//! `mod 2^256` 语义，也正是词级层所需要的。
//!
//! 之所以不用有符号整数：词级规范形 `Σ cᵢ·atomᵢ + k (mod 2ʷ)` 里，
//! 「减」表现为系数 `2ʷ − 1`，在 256 位下是完整宽度的值，`i128`/`u128` 都装不下。

use std::cmp::Ordering;

/// 小端 limbs：`limbs()[0]` 是最低 64 位。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct U256([u64; 4]);

impl U256 {
    pub const ZERO: U256 = U256([0; 4]);
    pub const ONE: U256 = U256([1, 0, 0, 0]);
    pub const MAX: U256 = U256([u64::MAX; 4]);

    pub const fn limbs(self) -> [u64; 4] {
        self.0
    }

    pub const fn from_u128(v: u128) -> U256 {
        U256([v as u64, (v >> 64) as u64, 0, 0])
    }

    /// 若值 < 2^128 则返回其 `u128`。
    pub fn to_u128(self) -> Option<u128> {
        if self.0[2] == 0 && self.0[3] == 0 {
            Some((self.0[0] as u128) | ((self.0[1] as u128) << 64))
        } else {
            None
        }
    }

    pub const fn is_zero(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    pub const fn is_one(self) -> bool {
        self.0[0] == 1 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    /// 低 `w` 位全 1（`w ≤ 256`）。
    pub fn mask(w: u32) -> U256 {
        if w >= 256 {
            return U256::MAX;
        }
        let mut l = [0u64; 4];
        let full = (w / 64) as usize;
        for i in 0..full {
            l[i] = u64::MAX;
        }
        let rem = w % 64;
        if rem > 0 {
            l[full] = (1u64 << rem) - 1;
        }
        U256(l)
    }

    /// 按位掩码（`mod 2^w`）。
    pub fn and_mask(self, w: u32) -> U256 {
        self.and(U256::mask(w))
    }

    /// `mod 2^256` 加法。
    pub fn add(self, o: U256) -> U256 {
        let mut r = [0u64; 4];
        let mut carry = 0u64;
        for i in 0..4 {
            let (s1, c1) = self.0[i].overflowing_add(o.0[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            r[i] = s2;
            carry = (c1 as u64) + (c2 as u64);
        }
        U256(r)
    }

    /// `mod 2^256` 减法（两补）。
    pub fn sub(self, o: U256) -> U256 {
        self.add(o.not()).add(U256::ONE)
    }

    pub fn not(self) -> U256 {
        U256([!self.0[0], !self.0[1], !self.0[2], !self.0[3]])
    }

    /// 截断到低 256 位的乘法（`mod 2^256`）。
    ///
    /// 教科书竖式；`cur` 的上界恰为 `2^128 − 1`，不溢出 `u128`。
    pub fn mul(self, o: U256) -> U256 {
        let mut r = [0u64; 4];
        for i in 0..4 {
            let mut carry: u128 = 0;
            for j in 0..(4 - i) {
                let cur =
                    r[i + j] as u128 + (self.0[i] as u128) * (o.0[j] as u128) + carry;
                r[i + j] = cur as u64;
                carry = cur >> 64;
            }
            // 溢出到第 5 个 limb 的进位被丢弃 = mod 2^256
        }
        U256(r)
    }

    pub fn and(self, o: U256) -> U256 {
        U256([
            self.0[0] & o.0[0],
            self.0[1] & o.0[1],
            self.0[2] & o.0[2],
            self.0[3] & o.0[3],
        ])
    }

    pub fn or(self, o: U256) -> U256 {
        U256([
            self.0[0] | o.0[0],
            self.0[1] | o.0[1],
            self.0[2] | o.0[2],
            self.0[3] | o.0[3],
        ])
    }

    pub fn xor(self, o: U256) -> U256 {
        U256([
            self.0[0] ^ o.0[0],
            self.0[1] ^ o.0[1],
            self.0[2] ^ o.0[2],
            self.0[3] ^ o.0[3],
        ])
    }
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in (0..4).rev() {
            match self.0[i].cmp(&other.0[i]) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_to_u128_roundtrip() {
        for v in [0u128, 1, u64::MAX as u128, u128::MAX] {
            assert_eq!(U256::from_u128(v).to_u128(), Some(v));
        }
        // 高位非零 → 不是 u128
        assert_eq!(U256::MAX.to_u128(), None);
        assert_eq!(U256([0, 0, 1, 0]).to_u128(), None);
    }

    #[test]
    fn mask_shapes() {
        assert_eq!(U256::mask(0), U256::ZERO);
        assert_eq!(U256::mask(8), U256::from_u128(0xFF));
        assert_eq!(U256::mask(64), U256([u64::MAX, 0, 0, 0]));
        assert_eq!(U256::mask(65), U256([u64::MAX, 1, 0, 0]));
        assert_eq!(U256::mask(256), U256::MAX);
        assert_eq!(U256::mask(300), U256::MAX);
    }

    #[test]
    fn add_carries_across_limbs() {
        let a = U256::from_u128(u128::MAX);
        let b = U256::ONE;
        let s = a.add(b);
        assert_eq!(s.limbs(), [0, 0, 1, 0], "u128::MAX + 1 应进位到第 3 个 limb");
        // 环绕
        assert_eq!(U256::MAX.add(U256::ONE), U256::ZERO);
    }

    #[test]
    fn sub_is_twos_complement() {
        assert_eq!(U256::ONE.sub(U256::ONE), U256::ZERO);
        assert_eq!(U256::ZERO.sub(U256::ONE), U256::MAX);
        // 关键性质：0 - 1 + 1 == 0（mod 2^256）
        assert_eq!(U256::ZERO.sub(U256::ONE).add(U256::ONE), U256::ZERO);
        // 跨 limb 借位
        let a = U256([0, 0, 1, 0]);
        assert_eq!(a.sub(U256::ONE).limbs(), [u64::MAX, u64::MAX, 0, 0]);
    }

    #[test]
    fn mul_truncates_mod_2_256() {
        assert_eq!(U256::from_u128(3).mul(U256::from_u128(5)), U256::from_u128(15));
        // (2^255) * 2 == 0 mod 2^256
        let two_255 = U256([0, 0, 0, 1u64 << 63]);
        assert_eq!(two_255.mul(U256::from_u128(2)), U256::ZERO);
        // (2^128-1)^2 mod 2^256 的正确值
        let m = U256::from_u128(u128::MAX);
        let sq = m.mul(m);
        // (2^128-1)^2 = 2^256 - 2^129 + 1  →  低 256 位是 1 - 2^129 mod 2^256
        let expect = U256::ONE.sub(U256([0, 0, 2, 0]));
        assert_eq!(sq, expect);
    }

    #[test]
    fn key_cancellation_property() {
        // 词级层赖以成立的核心：(-1) + 1 ≡ 0 (mod 2^256)
        let minus_one = U256::mask(256);
        assert_eq!(minus_one.add(U256::ONE).and_mask(256), U256::ZERO);
    }

    #[test]
    fn ordering_is_numeric_not_lexicographic() {
        let small = U256::from_u128(5);
        let big = U256([0, 1, 0, 0]); // 2^64
        assert!(small < big, "低位 limb 大不代表数值大");
        assert!(U256::ZERO < U256::ONE);
        assert!(U256::MAX > big);
    }

    #[test]
    fn bitwise_ops() {
        let a = U256::from_u128(0b1100);
        let b = U256::from_u128(0b1010);
        assert_eq!(a.and(b), U256::from_u128(0b1000));
        assert_eq!(a.or(b), U256::from_u128(0b1110));
        assert_eq!(a.xor(b), U256::from_u128(0b0110));
    }
}
