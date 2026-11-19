use highfleet::general::CVec;
use serde::{Serialize, Serializer};

use crate::structs::loadout::GameLoadout;

#[repr(C)]
#[derive(Debug, Default, Clone)]
pub struct Plane {
    pub _padding: [u8; 8],
    pub loadouts: CVec<*const GameLoadout>,
}

const _: () = {
    use crate::structs::loadout::GameItemMunition;
    use highfleet::general::{EscadraString, Tll, TllContainer};
    use std::mem::{offset_of, size_of};

    assert!(size_of::<Plane>() == 0x20);
    assert!(offset_of!(Plane, loadouts) == 0x08);
    assert!(size_of::<GameItemMunition>() == 0x28);
    assert!(offset_of!(GameItemMunition, count) == 0x20);
    assert!(size_of::<GameLoadout>() == 0x60);
    assert!(offset_of!(GameLoadout, vec_parts) == 0x40);
    assert!(offset_of!(GameLoadout, launch_loadout_weight) == 0x58);
    assert!(offset_of!(GameLoadout, has_gun37mm) == 0x5c);
    type PlaneNode = Tll<EscadraString, Plane>;
    assert!(size_of::<TllContainer<EscadraString, Plane>>() == 0x10);
    assert!(size_of::<PlaneNode>() == 0x60);
    assert!(offset_of!(PlaneNode, key) == 0x20);
    assert!(offset_of!(PlaneNode, data) == 0x40);
};

impl Serialize for Plane {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("Plane", 2)?;

        state.serialize_field("_padding", &self._padding)?;

        // Convert the pointer vector to serializable game loadouts.
        let loadouts_vec: Vec<&GameLoadout> = self
            .loadouts
            .as_slice()
            .iter()
            .filter_map(|&loadout| unsafe { loadout.as_ref() })
            .collect();

        state.serialize_field("loadouts", &loadouts_vec)?;
        state.end()
    }
}
