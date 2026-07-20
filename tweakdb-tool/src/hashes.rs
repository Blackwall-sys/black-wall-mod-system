//! Hashes do tweakdb (CRC32/TweakDBID, murmur3 type-key, FNV-1a 32/64).
//! FONTE ÚNICA agora vive no crate `bwms-hashes` (compartilhado com o runtime cp77-console) —
//! aqui é só o re-export, pra `crate::hashes::*` seguir funcionando sem duplicar a implementação.
pub use bwms_hashes::{fnv1a32, fnv1a64, record_type_key, tweak_db_id, tweak_db_id_derive};
