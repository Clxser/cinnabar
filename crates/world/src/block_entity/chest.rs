//! Pairing coordinates carried by a newly loaded chest record.

use crate::{NbtCompound, NbtValue};

/// Reads the partner claimed by a fresh chest record, whose lead flag starts set.
///
/// This reads the record only; loaded-block compatibility and existing actor pairing
/// state determine whether the claim can become a pair.
/// `pairx` must be an Int, `pairz` defaults to zero unless it is an Int, and only a Byte
/// `pairlead` changes the default lead flag.
#[must_use]
pub fn chest_pair_partner(position: [i32; 3], nbt: &NbtCompound) -> Option<[i32; 3]> {
    let &NbtValue::Int(pair_x) = nbt.get("pairx")? else {
        return None;
    };
    if matches!(nbt.get("pairlead"), Some(NbtValue::Byte(0))) {
        return None;
    }
    let pair_z = match nbt.get("pairz") {
        Some(NbtValue::Int(value)) => *value,
        _ => 0,
    };
    Some([pair_x, position[1], pair_z])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a record without an explicit lead flag.
    fn record() -> NbtCompound {
        let mut nbt = NbtCompound::default();
        nbt.insert("pairx", NbtValue::Int(5));
        nbt.insert("pairz", NbtValue::Int(-3));
        nbt
    }

    #[test]
    fn fresh_records_lead_unless_a_byte_flag_clears_it() {
        let mut nbt = record();
        let position = [6, 64, -3];
        let partner = Some([5, 64, -3]);
        assert_eq!(chest_pair_partner(position, &nbt), partner);
        for value in [-1, 1] {
            nbt.insert("pairlead", NbtValue::Byte(value));
            assert_eq!(chest_pair_partner(position, &nbt), partner);
        }
        nbt.insert("pairlead", NbtValue::Byte(0));
        assert_eq!(chest_pair_partner(position, &nbt), None);
    }

    #[test]
    fn non_byte_lead_flags_leave_the_default_set() {
        for value in [
            NbtValue::Short(0),
            NbtValue::Int(0),
            NbtValue::Long(0),
            NbtValue::String("false".into()),
        ] {
            let mut nbt = record();
            nbt.insert("pairlead", value);
            assert_eq!(chest_pair_partner([6, 64, -3], &nbt), Some([5, 64, -3]));
        }
    }

    #[test]
    fn pair_x_requires_an_int_tag() {
        for value in [
            NbtValue::Byte(5),
            NbtValue::Short(5),
            NbtValue::Long(5),
            NbtValue::Float(5.0),
        ] {
            let mut nbt = record();
            nbt.insert("pairx", value);
            assert_eq!(chest_pair_partner([6, 64, -3], &nbt), None);
        }
        assert_eq!(
            chest_pair_partner([6, 64, -3], &NbtCompound::default()),
            None
        );
    }

    #[test]
    fn missing_or_non_int_pair_z_defaults_to_zero() {
        let mut nbt = NbtCompound::default();
        nbt.insert("pairx", NbtValue::Int(5));
        assert_eq!(chest_pair_partner([6, 64, 0], &nbt), Some([5, 64, 0]));
        for value in [
            NbtValue::Byte(-3),
            NbtValue::Short(-3),
            NbtValue::Long(-3),
            NbtValue::String("-3".into()),
        ] {
            nbt.insert("pairz", value);
            assert_eq!(chest_pair_partner([6, 64, 0], &nbt), Some([5, 64, 0]));
        }
    }
}
