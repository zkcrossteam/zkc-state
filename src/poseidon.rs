use ff::PrimeField;
use halo2_proofs::pairing::bn256::Fr;
use poseidon::Poseidon;

use crate::errors::Error;

pub const PREFIX_CHALLENGE: u64 = 0u64;
pub const PREFIX_POINT: u64 = 1u64;
pub const PREFIX_SCALAR: u64 = 2u64;

/// There are three variants of haser used in upstream.
/// https://github.com/DelphinusLab/zkWasm-host-circuits/blob/e3a2eff4583b2fd8be7fc3e54f2789cbfbfd72d4/src/host/poseidon.rs#L9-L20
/// This function creates a hasher equivalent to the MERKLE_LEAF_HASHER.
/// ```text
/// // We have two hasher here
/// // 1. MERKLE_HASHER that is used for non sponge hash for hash two merkle siblings
/// // 2. POSEIDON_HASHER thas is use for poseidon hash of data
/// ```
///
/// ```rust,ignore
/// lazy_static::lazy_static! {
///     pub static ref POSEIDON_HASHER: poseidon::Poseidon<Fr, 9, 8> = Poseidon::<Fr, 9, 8>::new(8, 63);
///     pub static ref MERKLE_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref MERKLE_LEAF_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref POSEIDON_HASHER_SPEC: poseidon::Spec<Fr, 9, 8> = Spec::new(8, 63);
///     pub static ref MERKLE_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
///     pub static ref MERKLE_LEAF_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
/// }
/// ```
pub fn gen_poseidon_hasher() -> Poseidon<Fr, 3, 2> {
    Poseidon::<Fr, 3, 2>::new(8, 57)
}

/// There are three variants of haser used in upstream.
/// https://github.com/DelphinusLab/zkWasm-host-circuits/blob/e3a2eff4583b2fd8be7fc3e54f2789cbfbfd72d4/src/host/poseidon.rs#L9-L20
/// This function creates a hasher equivalent to the MERKLE_HASHER.
/// ```text
/// // We have two hasher here
/// // 1. MERKLE_HASHER that is used for non sponge hash for hash two merkle siblings
/// // 2. POSEIDON_HASHER thas is use for poseidon hash of data
/// ```
///
/// ```rust,ignore
/// lazy_static::lazy_static! {
///     pub static ref POSEIDON_HASHER: poseidon::Poseidon<Fr, 9, 8> = Poseidon::<Fr, 9, 8>::new(8, 63);
///     pub static ref MERKLE_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref MERKLE_LEAF_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref POSEIDON_HASHER_SPEC: poseidon::Spec<Fr, 9, 8> = Spec::new(8, 63);
///     pub static ref MERKLE_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
///     pub static ref MERKLE_LEAF_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
/// }
/// ```
pub fn gen_merkle_hasher() -> Poseidon<Fr, 3, 2> {
    Poseidon::<Fr, 3, 2>::new(8, 57)
}

pub fn hash_field_elements(frs: &[Fr]) -> <Fr as PrimeField>::Repr {
    dbg!(frs);
    let mut hasher = gen_poseidon_hasher();
    hasher.update(frs);
    let hash = hasher.squeeze().to_repr();
    dbg!(&hash);
    hash
}

/// Hash data from an array of 32 bytes. Since we will split each 32 bytes to
/// two 16 bytes and convert them into field elements, we do not require each
/// 32 bytes to be a valid field element.
/// If feature feature="complex-leaf" is not enabled, then this is the default
/// hashing behaviour of zkWasm-host-circuits.
/// https://github.com/DelphinusLab/zkWasm-host-circuits/pull/75/files
pub fn hash_with_padding(data_to_hash: &[u8]) -> Result<<Fr as PrimeField>::Repr, Error> {
    let num_of_bytes: usize = 32;
    if data_to_hash.len() % num_of_bytes != 0 {
        return Err(Error::InvalidArgument(
            "Invalid data to hash, must be an array of field elements".to_string(),
        ));
    }
    let frs = data_to_hash
        .chunks(16)
        .map(|x| {
            let mut v = x.clone().to_vec();
            v.extend_from_slice(&[0u8; 16]);
            let f = v.try_into().unwrap();
            Fr::from_repr(f).unwrap()
        })
        .collect::<Vec<Fr>>();
    dbg!(&frs);
    Ok(hash_field_elements(&frs))
}

/// Hash data from an array of 32 bytes. Each 32 bytes must be a valid field element.
pub fn hash(data_to_hash: &[u8]) -> Result<<Fr as PrimeField>::Repr, Error> {
    dbg!(data_to_hash);
    let num_of_bytes: usize = 32;
    if data_to_hash.len() % num_of_bytes != 0 {
        return Err(Error::InvalidArgument(
            "Invalid data to hash, must be an array of field elements".to_string(),
        ));
    }
    let frs = data_to_hash
        .chunks(num_of_bytes)
        .map(|x| {
            let v = x.try_into().unwrap();
            let f = Fr::from_repr(v);
            if f.is_none().into() {
                return Err(Error::InvalidArgument(
                    "Invalid data to hash, must be an array of field elements".to_string(),
                ));
            }
            Ok(f.unwrap())
        })
        .collect::<Result<Vec<Fr>, _>>()?;
    Ok(hash_field_elements(&frs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff::PrimeField;
    use halo2_proofs::pairing::bn256::Fr;

    #[test]
    fn test_merkle_hash_zero() {
        const ZERO_HASHER_SQUEEZE: &str =
            "0x0ac6c5f29f5187473a70dfde3329ef18f01a4d84edb01e6c21813f629a6b5f50";
        let mut hasher = super::gen_poseidon_hasher();
        hasher.update(&[Fr::zero()]);
        let result = hasher.squeeze();
        println!("hash result is {:?}", result);
        assert_eq!(result.to_string(), ZERO_HASHER_SQUEEZE);
    }

    #[test]
    fn test_poseidon_hash_zero() {
        const ZERO_HASHER_SQUEEZE: &str =
            "0x0ac6c5f29f5187473a70dfde3329ef18f01a4d84edb01e6c21813f629a6b5f50";
        let mut hasher = super::gen_poseidon_hasher();
        hasher.update(&[Fr::zero()]);
        let result = hasher.squeeze();
        println!("hash result is {:?}", result);
        assert_eq!(result.to_string(), ZERO_HASHER_SQUEEZE);
    }

    #[test]
    fn test_poseidon_hash_equivalent() {
        let mut hasher = super::gen_poseidon_hasher();
        hasher.update(&[Fr::zero()]);
        let result = hasher.squeeze().to_repr();
        println!("hash result is {:?}", result);
        let result2 = hash(&[0; 32]).expect("Hash succeeded");
        assert_eq!(result, result2);
    }

    #[test]
    fn test_poseidon_hash_with_padding_equivalent() {
        let mut hasher = super::gen_poseidon_hasher();
        hasher.update(&[Fr::zero(), Fr::zero()]);
        let result = hasher.squeeze().to_repr();
        println!("hash result is {:?}", result);
        let result2 = hash_with_padding(&[0; 32]).expect("Hash succeeded");
        assert_eq!(result, result2);
    }
}
