use ff::PrimeField;
use halo2_proofs::pairing::bn256::Fr;
use poseidon::Poseidon;

use crate::errors::Error;

pub const PREFIX_CHALLENGE: u64 = 0u64;
pub const PREFIX_POINT: u64 = 1u64;
pub const PREFIX_SCALAR: u64 = 2u64;

/// There is two variants of haser used in upstream.
/// This is the POSEIDON_HASHER
/// https://github.com/DelphinusLab/zkWasm-host-circuits/blob/f0bae8b70c33941d6969635e4b1bba012441ea4d/src/host/poseidon.rs#L9-L17
/// ```text
/// We have two hasher here
/// 1. MERKLE_HASHER that is used for non sponge hash for hash two merkle siblings
/// 2. POSEIDON_HASHER thas is use for poseidon hash of data
/// ```
///
/// ```rust,ignore
/// lazy_static::lazy_static! {
///     pub static ref POSEIDON_HASHER: poseidon::Poseidon<Fr, 9, 8> = Poseidon::<Fr, 9, 8>::new(8, 63);
///     pub static ref MERKLE_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref POSEIDON_HASHER_SPEC: poseidon::Spec<Fr, 9, 8> = Spec::new(8, 63);
///     pub static ref MERKLE_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
/// }
/// ```
pub fn gen_poseidon_hasher() -> Poseidon<Fr, 9, 8> {
    Poseidon::<Fr, 9, 8>::new(8, 63)
}

/// There is two variants of haser used in upstream.
/// This is the MERKLE_HASHER
/// https://github.com/DelphinusLab/zkWasm-host-circuits/blob/f0bae8b70c33941d6969635e4b1bba012441ea4d/src/host/poseidon.rs#L9-L17
/// ```text
/// We have two hasher here
/// 1. MERKLE_HASHER that is used for non sponge hash for hash two merkle siblings
/// 2. POSEIDON_HASHER thas is use for poseidon hash of data
/// ```
///
/// ```rust,ignore
/// lazy_static::lazy_static! {
///     pub static ref POSEIDON_HASHER: poseidon::Poseidon<Fr, 9, 8> = Poseidon::<Fr, 9, 8>::new(8, 63);
///     pub static ref MERKLE_HASHER: poseidon::Poseidon<Fr, 3, 2> = Poseidon::<Fr, 3, 2>::new(8, 57);
///     pub static ref POSEIDON_HASHER_SPEC: poseidon::Spec<Fr, 9, 8> = Spec::new(8, 63);
///     pub static ref MERKLE_HASHER_SPEC: poseidon::Spec<Fr, 3, 2> = Spec::new(8, 57);
/// }
/// ```
pub fn gen_merkle_hasher() -> Poseidon<Fr, 3, 2> {
    Poseidon::<Fr, 3, 2>::new(8, 57)
}

pub fn hash(data_to_hash: &[u8]) -> Result<<Fr as PrimeField>::Repr, Error> {
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
    let mut hasher = gen_poseidon_hasher();
    hasher.update(&frs);
    let hash = hasher.squeeze().to_repr();
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use halo2_proofs::pairing::bn256::Fr;
    #[test]
    fn test_poseidon() {
        const ZERO_HASHER_SQUEEZE: &str =
            "0x03f943aabd67cd7b72a539f3de686c3280c36c572be09f2b9193f5ef78761c6b"; //force the hasher is for fr field result.
        let mut hasher = super::gen_poseidon_hasher();
        hasher.update(&[Fr::zero()]);
        let result = hasher.squeeze();
        println!("hash result is {:?}", result);
        assert_eq!(result.to_string(), ZERO_HASHER_SQUEEZE);
    }
}
