use ff::PrimeField;
use halo2_proofs::pairing::bn256::Fr;
use poseidon::Poseidon;

use crate::errors::Error;

pub const T: usize = 9;
pub const RATE: usize = 8;
pub const R_F: usize = 8;
pub const R_P: usize = 63;

pub const PREFIX_CHALLENGE: u64 = 0u64;
pub const PREFIX_POINT: u64 = 1u64;
pub const PREFIX_SCALAR: u64 = 2u64;

pub fn gen_hasher() -> Poseidon<Fr, T, RATE> {
    Poseidon::<Fr, T, RATE>::new(R_F, R_P)
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
    let mut hasher = gen_hasher();
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
        let mut hasher = super::gen_hasher();
        hasher.update(&[Fr::zero()]);
        let result = hasher.squeeze();
        println!("hash result is {:?}", result);
        assert_eq!(result.to_string(), ZERO_HASHER_SQUEEZE);
    }
}
