//! Dense, typed ids for every entity.
//!
//! Entities live in struct-of-arrays storage (ARCHITECTURE §3) and are
//! indexed by these ids. Newtypes stop a robot index being used as a parcel
//! index, and dense integer ids give a total order for deterministic
//! tie-breaks (ADR 0002).

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! dense_id {
    ($(#[$meta:meta])* $name:ident($repr:ty), $prefix:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        #[repr(transparent)]
        pub struct $name(pub $repr);

        impl $name {
            /// The id as a `usize` index into the owning struct-of-arrays.
            #[inline]
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

dense_id!(
    /// A robot (sorting AMR).
    RobotId(u32),
    "R"
);
dense_id!(
    /// A parcel. Ids are assigned in arrival order.
    ParcelId(u32),
    "P"
);
dense_id!(
    /// A chute (a hole in the floor that a robot drops parcels into).
    ChuteId(u16),
    "C"
);
dense_id!(
    /// An induction station on the floor edge.
    StationId(u16),
    "S"
);
dense_id!(
    /// A sort destination (for example a postcode group). In V0 each chute
    /// serves exactly one destination with the same index.
    DestinationId(u16),
    "D"
);
dense_id!(
    /// An entry in the fault schedule: its index in the scenario's
    /// `[[faults]]` list, then in [`crate::sim::Sim::schedule_fault`] order.
    FaultId(u32),
    "F"
);
