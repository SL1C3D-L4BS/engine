//! Typed asset handles and the [`AssetServer`].
//!
//! Game and editor code never touches asset bytes directly — it loads an
//! asset by name and holds a [`Handle<T>`]. The [`AssetServer`] owns the
//! decoded asset, hands out cheap clonable handles, deduplicates repeat loads,
//! and supports hot-reload: when an asset's bytes change, every existing
//! handle observes the new value with no handle invalidation (spec IV.8).
//!
//! The byte source is a [`PakSet`], so the overlay and kill-switch behaviour
//! of Live Ops applies transparently. Hot-reload is driven by re-resolving
//! against that source — mount an update pak (or have the filesystem
//! [`watch`](engine_platform::watch) layer signal a change) and call
//! [`AssetServer::reload`].

use crate::pak::PakSet;
use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// A type that can be decoded from raw asset bytes.
pub trait Asset: Send + Sync + 'static {
    /// Decodes the asset from its on-disk byte form.
    fn decode(bytes: &[u8]) -> Result<Self, AssetError>
    where
        Self: Sized;
}

/// Why an asset could not be loaded or reloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// No mounted pak provides the requested name (or it is kill-switched).
    NotFound,
    /// The bytes were found but could not be decoded into the asset type.
    Decode(String),
    /// The name is cached under a different asset type than the one requested.
    TypeMismatch,
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "asset not found"),
            Self::Decode(why) => write!(f, "asset decode failed: {why}"),
            Self::TypeMismatch => write!(f, "asset cached under a different type"),
        }
    }
}

impl std::error::Error for AssetError {}

/// The mutable cell holding the current value of one decoded asset. Hot-reload
/// swaps the inner `Arc`; handles keep pointing at the cell.
struct Slot<T> {
    current: Mutex<Arc<T>>,
}

/// Type-erased view of a [`Slot`] so the server can reload any asset without
/// knowing its concrete type.
trait AnySlot: Send + Sync + 'static {
    fn reload_from(&self, bytes: &[u8]) -> Result<(), AssetError>;
    fn as_any(&self) -> &dyn Any;
}

impl<T: Asset> AnySlot for Slot<T> {
    fn reload_from(&self, bytes: &[u8]) -> Result<(), AssetError> {
        let value = T::decode(bytes)?;
        *self.current.lock().unwrap() = Arc::new(value);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A cheap, clonable, typed reference to a loaded asset.
///
/// Cloning a handle is a pointer-copy and a refcount bump. [`get`](Self::get)
/// returns the *current* value — after a hot-reload it returns the new one.
pub struct Handle<T> {
    name: Arc<str>,
    slot: Arc<dyn AnySlot>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            slot: Arc::clone(&self.slot),
            _marker: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("name", &self.name)
            .field("type", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T: Asset> Handle<T> {
    /// The current decoded value. The returned `Arc` is a stable snapshot — a
    /// concurrent hot-reload does not mutate it, only future calls see the
    /// newer value.
    pub fn get(&self) -> Arc<T> {
        let slot = self
            .slot
            .as_any()
            .downcast_ref::<Slot<T>>()
            .expect("Handle<T> always wraps a Slot<T>");
        Arc::clone(&slot.current.lock().unwrap())
    }
}

impl<T> Handle<T> {
    /// The logical name this handle was loaded under.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Loads, caches, and hot-reloads assets from a [`PakSet`].
#[derive(Default)]
pub struct AssetServer {
    source: PakSet,
    cache: HashMap<String, Arc<dyn AnySlot>>,
}

impl AssetServer {
    /// Creates a server with an empty pak set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a server over an already-populated pak set.
    pub fn with_source(source: PakSet) -> Self {
        Self {
            source,
            cache: HashMap::new(),
        }
    }

    /// The pak set assets are resolved from — mount update paks here, then
    /// call [`reload`](Self::reload) to pick the new bytes up live.
    pub fn source_mut(&mut self) -> &mut PakSet {
        &mut self.source
    }

    /// Loads `name` as a `T`, decoding and caching it on first request and
    /// returning a fresh handle to the cached value thereafter.
    pub fn load<T: Asset>(&mut self, name: &str) -> Result<Handle<T>, AssetError> {
        if let Some(slot) = self.cache.get(name) {
            if slot.as_any().downcast_ref::<Slot<T>>().is_none() {
                return Err(AssetError::TypeMismatch);
            }
            return Ok(Handle {
                name: Arc::from(name),
                slot: Arc::clone(slot),
                _marker: PhantomData,
            });
        }

        let bytes = self.source.resolve(name).ok_or(AssetError::NotFound)?;
        let value = T::decode(bytes)?;
        let slot: Arc<dyn AnySlot> = Arc::new(Slot {
            current: Mutex::new(Arc::new(value)),
        });
        self.cache.insert(name.to_string(), Arc::clone(&slot));
        Ok(Handle {
            name: Arc::from(name),
            slot,
            _marker: PhantomData,
        })
    }

    /// Re-resolves `name` against the pak set and swaps the new value into the
    /// existing slot — every outstanding [`Handle`] sees the update.
    ///
    /// Errors if the name was never loaded, no longer resolves, or fails to
    /// decode; in every error case the previously loaded value is left intact.
    pub fn reload(&self, name: &str) -> Result<(), AssetError> {
        let slot = self.cache.get(name).ok_or(AssetError::NotFound)?;
        let bytes = self.source.resolve(name).ok_or(AssetError::NotFound)?;
        slot.reload_from(bytes)
    }

    /// Number of handles outstanding for `name`, excluding the server's own
    /// cache reference. Zero means only the cache still holds the asset.
    pub fn handle_count(&self, name: &str) -> usize {
        self.cache
            .get(name)
            .map_or(0, |slot| Arc::strong_count(slot).saturating_sub(1))
    }

    /// Drops cached assets that no live handle still references — the
    /// ref-count-driven half of asset memory management.
    pub fn evict_unused(&mut self) {
        self.cache.retain(|_, slot| Arc::strong_count(slot) > 1);
    }

    /// Number of distinct assets currently cached.
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pak::Pak;

    /// A trivial asset: the byte length of its source.
    #[derive(Debug, PartialEq, Eq)]
    struct ByteCount(usize);

    impl Asset for ByteCount {
        fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
            Ok(ByteCount(bytes.len()))
        }
    }

    fn server_with(name: &str, payload: &[u8]) -> AssetServer {
        let mut b = Pak::builder();
        b.add(name, payload.to_vec());
        let mut set = PakSet::new();
        set.mount(b.build());
        AssetServer::with_source(set)
    }

    #[test]
    fn load_decodes_and_caches() {
        let mut server = server_with("a.bin", b"hello");
        let h = server.load::<ByteCount>("a.bin").unwrap();
        assert_eq!(*h.get(), ByteCount(5));
        // Second load is a cache hit, not a re-decode of a new slot.
        let h2 = server.load::<ByteCount>("a.bin").unwrap();
        assert_eq!(server.cached_count(), 1);
        assert!(Arc::ptr_eq(&h.get(), &h2.get()));
    }

    #[test]
    fn missing_asset_errors() {
        let mut server = server_with("a.bin", b"x");
        assert_eq!(
            server.load::<ByteCount>("missing").unwrap_err(),
            AssetError::NotFound
        );
    }

    #[test]
    fn hot_reload_updates_live_handles() {
        let mut server = server_with("cfg", b"v1");
        let handle = server.load::<ByteCount>("cfg").unwrap();
        assert_eq!(*handle.get(), ByteCount(2));

        // Mount an update pak — the Live Ops overlay — then reload.
        let mut patch = Pak::builder();
        patch.add("cfg", b"version-2".to_vec());
        server.source_mut().mount(patch.build());
        server.reload("cfg").unwrap();

        // The handle held since before the reload sees the new value.
        assert_eq!(*handle.get(), ByteCount(9));
    }

    #[test]
    fn ref_count_drives_eviction() {
        let mut server = server_with("a.bin", b"data");
        let handle = server.load::<ByteCount>("a.bin").unwrap();
        assert_eq!(server.handle_count("a.bin"), 1);

        server.evict_unused();
        assert_eq!(server.cached_count(), 1); // handle still live

        drop(handle);
        assert_eq!(server.handle_count("a.bin"), 0);
        server.evict_unused();
        assert_eq!(server.cached_count(), 0);
    }

    #[test]
    fn loading_under_the_wrong_type_is_rejected() {
        struct Other(());
        impl Asset for Other {
            fn decode(_: &[u8]) -> Result<Self, AssetError> {
                Ok(Other(()))
            }
        }
        let mut server = server_with("a.bin", b"data");
        server.load::<ByteCount>("a.bin").unwrap();
        assert_eq!(
            server.load::<Other>("a.bin").unwrap_err(),
            AssetError::TypeMismatch
        );
    }
}
