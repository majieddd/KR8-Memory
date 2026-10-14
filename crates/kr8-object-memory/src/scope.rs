//! Trusted profile identity and exact-place read grants.

use crate::{bounded, Digest, MemoryError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Full authenticated identity persisted in each profile database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileIdentity {
    owner: String,
    relay: String,
    community: String,
}

impl ProfileIdentity {
    /// Construct from authenticated native state, never a request DTO.
    pub fn new(owner: &str, relay: &str, community: &str) -> Result<Self, MemoryError> {
        bounded(owner, 256, "owner")?;
        bounded(community, 512, "community")?;
        let relay = url::Url::parse(relay).map_err(|_| MemoryError::Invalid("relay"))?;
        if !matches!(relay.scheme(), "ws" | "wss" | "http" | "https")
            || relay.host_str().is_none()
            || !relay.username().is_empty()
            || relay.password().is_some()
            || relay.query().is_some()
            || relay.fragment().is_some()
        {
            return Err(MemoryError::Invalid("relay"));
        }
        let relay = if relay.path() == "/" {
            relay.as_str().trim_end_matches('/').to_owned()
        } else {
            relay.as_str().to_owned()
        };
        bounded(&relay, 2048, "relay")?;
        Ok(Self {
            owner: owner.to_owned(),
            relay,
            community: community.to_owned(),
        })
    }

    /// Authenticated owner key, not a display name.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Canonical relay URL.
    pub fn relay(&self) -> &str {
        &self.relay
    }

    /// Authenticated community identity.
    pub fn community(&self) -> &str {
        &self.community
    }

    /// Canonical full identity for strict database binding checks.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, MemoryError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Stable Windows-safe partition key over the entire identity tuple.
    pub fn key(&self) -> Result<ProfileKey, MemoryError> {
        Ok(ProfileKey(Digest::of(&self.canonical_bytes()?)))
    }
}

/// Opaque SHA-256 profile partition. Its display form is path-safe hex only.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileKey(Digest);

impl ProfileKey {
    /// Parse an exact canonical partition key from trusted persisted metadata.
    pub fn parse(value: &str) -> Result<Self, MemoryError> {
        Ok(Self(Digest::parse(value)?))
    }
    /// The safe 64-character directory segment.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A precise storage place within one profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceId {
    profile: ProfileKey,
    kind: PlaceKind,
}

/// Initial owner-private or authenticated conversation place.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum PlaceKind {
    /// Visible to the owner workflow only; excluded from the initial ACP reader.
    OwnerPrivate,
    /// Exact native-resolved conversation ID.
    Conversation(String),
}

impl PlaceId {
    /// The owner-private namespace of one profile.
    pub fn owner_private(profile: ProfileKey) -> Self {
        Self {
            profile,
            kind: PlaceKind::OwnerPrivate,
        }
    }
    /// A native-verified conversation place. The channel ID cannot be a path.
    pub fn conversation(profile: ProfileKey, channel: &str) -> Result<Self, MemoryError> {
        bounded(channel, 512, "conversation")?;
        Ok(Self {
            profile,
            kind: PlaceKind::Conversation(channel.to_owned()),
        })
    }
    /// Bound profile partition.
    pub fn profile(&self) -> &ProfileKey {
        &self.profile
    }
    /// Exact kind and identifier.
    pub fn kind(&self) -> &PlaceKind {
        &self.kind
    }
    /// Stable canonical JSON identifier used as an opaque provider scope label.
    pub fn canonical(&self) -> Result<String, MemoryError> {
        Ok(serde_json::to_string(self)?)
    }
    /// Validate a deserialized place before use at a persistence boundary.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if let PlaceKind::Conversation(id) = &self.kind {
            bounded(id, 512, "conversation")?;
        }
        Ok(())
    }
    /// Parse a persisted/provider label without accepting noncanonical aliases.
    pub fn parse(value: &str) -> Result<Self, MemoryError> {
        let place: Self = serde_json::from_str(value)?;
        place.validate()?;
        if place.canonical()? != value {
            return Err(MemoryError::Invalid("noncanonical place"));
        }
        Ok(place)
    }
}

/// Native-derived exact allowed-place set. Deliberately not deserializable.
#[derive(Clone, Debug)]
pub struct ReadScope {
    profile: ProfileKey,
    places: BTreeSet<PlaceId>,
}

impl ReadScope {
    /// Rebuild the provider contract's already-authorized native place set.
    /// This validates identity/format only; the caller must freshly verify
    /// membership before constructing a ScopedQuery. No request DTO can decode it.
    pub fn from_authorized_places(
        profile: ProfileKey,
        authorized: impl IntoIterator<Item = PlaceId>,
    ) -> Result<Self, MemoryError> {
        let mut places = BTreeSet::new();
        for (position, place) in authorized.into_iter().enumerate() {
            if position >= 32 {
                return Err(MemoryError::Invalid("too many read places"));
            }
            place.validate()?;
            if place.profile() != &profile {
                return Err(MemoryError::ScopeMismatch);
            }
            places.insert(place);
        }
        if places.is_empty() {
            return Err(MemoryError::Invalid("empty read places"));
        }
        Ok(Self { profile, places })
    }

    /// Build the owner's current grants from freshly verified memberships.
    pub fn owner(
        profile: ProfileKey,
        conversations: impl IntoIterator<Item = PlaceId>,
    ) -> Result<Self, MemoryError> {
        let mut places = BTreeSet::from([PlaceId::owner_private(profile.clone())]);
        for place in conversations {
            place.validate()?;
            if place.profile() != &profile || !matches!(place.kind(), PlaceKind::Conversation(_)) {
                return Err(MemoryError::ScopeMismatch);
            }
            places.insert(place);
            if places.len() > 32 {
                return Err(MemoryError::Invalid("too many read places"));
            }
        }
        Ok(Self { profile, places })
    }
    /// The first ACP reader can receive exactly its authenticated conversation.
    pub fn exact_conversation(profile: ProfileKey, channel: &str) -> Result<Self, MemoryError> {
        let place = PlaceId::conversation(profile.clone(), channel)?;
        Ok(Self {
            profile,
            places: BTreeSet::from([place]),
        })
    }
    /// Bound profile partition.
    pub fn profile(&self) -> &ProfileKey {
        &self.profile
    }
    /// Already-authorized places, in stable order.
    pub fn places(&self) -> &BTreeSet<PlaceId> {
        &self.places
    }
    /// Test exact place access before ranking, limits, or text rendering.
    pub fn permits(&self, place: &PlaceId) -> bool {
        place.profile() == &self.profile && self.places.contains(place)
    }
}
