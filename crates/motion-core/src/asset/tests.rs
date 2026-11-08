//! Unit tests for asset loading, caching, request diagnostics and resource pools.

use super::*;

#[test]
fn configurable_pool_limits_cover_encoded_decoded_and_preloaded_payloads() -> crate::Result {
    struct Value;

    impl Asset for Value {
        const KIND: &'static str = "limited-value";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            Ok(Self)
        }

        fn retained_bytes(&self) -> usize {
            8
        }
    }
    assert!(
        ResourcePool::with_limits(crate::CacheLimits {
            entries: 1,
            bytes: 0
        })
        .is_err()
    );
    assert!(
        ResourcePool::with_limits(crate::CacheLimits {
            entries: 0,
            bytes: 8
        })
        .is_err()
    );
    assert!(
        ResourcePool::with_limits(crate::CacheLimits {
            entries: 4097,
            bytes: 8
        })
        .is_err()
    );
    let pool = ResourcePool::with_limits(crate::CacheLimits {
        entries: 1,
        bytes: 8,
    })?;
    assert_eq!(pool.max_bytes(), 8);
    assert_eq!(pool.max_entries(), 1);
    let assets = Assets::new(".")?.with_pool(pool.clone());
    assets.preload("first", Arc::new(Value))?;
    assert!(assets.preload("second", Arc::new(Value)).is_err());
    assert_eq!(pool.retained_bytes(), 8);
    let assets = Assets::new(".")?.with_pool(ResourcePool::with_limits(crate::CacheLimits {
        entries: 2,
        bytes: 7,
    })?);
    assert!(assets.preload("first", Arc::new(Value)).is_err());
    assert_eq!(assets.pool().retained_bytes(), 0);
    let assets = Assets::default().with_pool(ResourcePool::with_limits(crate::CacheLimits {
        entries: 2,
        bytes: 8,
    })?);
    let encoded = assets.load::<Value>(Source::Bytes {
        key: "encoded".into(),
        data: Arc::from(&b"123456789"[..]),
    });
    assert_eq!(encoded.status(), AssetStatus::Failed);
    assert_eq!(assets.pool().retained_bytes(), 0);
    let decoded = assets.load::<Value>(Source::Bytes {
        key: "decoded".into(),
        data: Arc::from(&b"1"[..]),
    });
    assert!(decoded.get().is_err());
    assert_eq!(assets.pool().retained_bytes(), 1);
    Ok(())
}

#[test]
fn removal_preserves_snapshots_handles_and_replacement_accounting() -> crate::Result {
    struct Value(u64);

    impl Asset for Value {
        const KIND: &'static str = "removable-value";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            unreachable!()
        }

        fn retained_bytes(&self) -> usize {
            8
        }
    }
    let pool = ResourcePool::with_limits(crate::CacheLimits {
        entries: 2,
        bytes: 16,
    })?;
    let assets = Assets::new(".")?.with_pool(pool.clone());
    let old = assets.preload("value", Arc::new(Value(1)))?;
    let snapshot = assets.snapshot();
    assert!(!assets.remove::<Value>("unknown"));
    assert!(assets.remove::<Value>("value"));
    assert!(!assets.remove::<Value>("value"));
    assert_eq!(old.get()?.0, 1);
    assert_eq!(snapshot.load::<Value>("value").get()?.0, 1);
    let new = assets.preload("value", Arc::new(Value(2)))?;
    assert_eq!(assets.load::<Value>("value").get()?.0, 2);
    assert_eq!(pool.retained_bytes(), 16);
    drop(old);
    drop(snapshot);
    assert_eq!(pool.retained_bytes(), 8);
    assert_eq!(new.get()?.0, 2);
    assert!(assets.remove::<Value>("value"));
    drop(new);
    assert_eq!(pool.retained_bytes(), 0);
    Ok(())
}

#[test]
fn preload_populates_existing_lazy_handle_without_decoding_or_replacing() -> crate::Result {
    struct Preloaded(u64);

    impl Asset for Preloaded {
        const KIND: &'static str = "preloaded";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            panic!("preloaded fixture must not read or decode")
        }
    }
    let assets = Assets::new(".")?;
    let before = assets.load::<Preloaded>("nonexistent.bin");
    assert_eq!(before.status(), AssetStatus::Unloaded);
    let value = Arc::new(Preloaded(42));
    let registered = assets.preload("nonexistent.bin", value.clone())?;
    assert_eq!(before.status(), AssetStatus::Ready);
    assert!(Arc::ptr_eq(&value, &before.get()?));
    assert!(Arc::ptr_eq(
        &registered.entry,
        &assets.clone().load::<Preloaded>("nonexistent.bin").entry
    ));
    assert!(
        assets
            .preload("nonexistent.bin", Arc::new(Preloaded(99)))
            .is_err()
    );
    assert_eq!(before.get()?.0, 42);
    Ok(())
}

#[test]
fn snapshot_preloading_does_not_resolve_an_old_unloaded_handle() -> crate::Result {
    struct Value;

    impl Asset for Value {
        const KIND: &'static str = "snapshot-value";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            panic!("snapshot preload must not decode")
        }
    }
    let assets = Assets::new(".")?;
    let original = assets.load::<Value>("unread.bin");
    let snapshot = assets.snapshot();
    snapshot.preload("unread.bin", Arc::new(Value))?;
    assert_eq!(
        snapshot.load::<Value>("unread.bin").status(),
        AssetStatus::Ready
    );
    assert_eq!(original.status(), AssetStatus::Unloaded);
    Ok(())
}

#[test]
fn preloads_keep_one_charge_after_owner_drop() -> crate::Result {
    struct Value;

    impl Asset for Value {
        const KIND: &'static str = "preloaded-value";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            panic!("preload must not decode")
        }

        fn retained_bytes(&self) -> usize {
            64
        }
    }
    let value = Arc::new(Value);
    let pool = ResourcePool::new();
    let assets = Assets::new(".")?.with_pool(pool.clone());
    assets.preload("value", value.clone())?;
    assert!(assets.pool().same_pool(&pool));
    let handle = assets.load::<Value>("value");
    assert_eq!(handle.status(), AssetStatus::Ready);
    assert!(Arc::ptr_eq(&value, &handle.get()?));
    assert_eq!(pool.retained_bytes(), 64);
    drop(assets);
    assert_eq!(pool.retained_bytes(), 64);
    drop(handle);
    assert_eq!(pool.retained_bytes(), 0);
    Ok(())
}

#[test]
fn encoded_byte_charge_survives_decode_failure_and_is_not_charged_twice() -> crate::Result {
    struct Broken;

    impl Asset for Broken {
        const KIND: &'static str = "broken-fixture";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            Err(io::Error::other("bad payload").into())
        }
    }
    let directory = tempfile::tempdir()?;
    let assets = Assets::new(directory.path())?;
    let source = Source::Bytes {
        key: "broken".into(),
        data: Arc::from(&b"invalid bytes"[..]),
    };
    let handle = assets.load::<Broken>(source.clone());
    assert_eq!(handle.entry.charged.load(Ordering::Acquire), 13);
    assert!(handle.get().is_err());
    assert_eq!(handle.entry.charged.load(Ordering::Acquire), 13);
    let shared = assets.load::<Broken>(source);
    assert!(Arc::ptr_eq(&handle.entry, &shared.entry));
    assert_eq!(shared.entry.charged.load(Ordering::Acquire), 13);
    Ok(())
}

#[test]
fn dropping_resource_owners_releases_encoded_keys_and_large_files_fail_before_reading()
-> crate::Result {
    struct Payload;

    impl Asset for Payload {
        const KIND: &'static str = "ownership-fixture";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            Ok(Self)
        }
    }
    let directory = tempfile::tempdir()?;
    let assets = Assets::new(directory.path())?;
    let data: Arc<[u8]> = Arc::from(&b"owned encoded bytes"[..]);
    let weak = Arc::downgrade(&data);
    let handle = assets.load::<Payload>(Source::Bytes {
        key: "ownership".into(),
        data,
    });
    let _value = handle.get()?;
    drop(handle);
    assert!(weak.upgrade().is_some());
    drop(assets);
    assert!(weak.upgrade().is_none());
    let path = directory.path().join("oversized.bin");
    std::fs::File::create(&path)?.set_len(MAX_ENCODED as u64 + 1)?;
    let assets = Assets::new(directory.path())?;
    assert_eq!(
        assets
            .load::<Payload>("oversized.bin")
            .get()
            .err()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidData
    );
    Ok(())
}

#[test]
fn loading_status_is_observable_and_never_resets_before_publication() -> crate::Result {
    use std::sync::Barrier;

    static GATES: OnceLock<(Barrier, Barrier)> = OnceLock::new();

    struct Blocking;

    impl Asset for Blocking {
        const KIND: &'static str = "loading-fixture";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            let (start, finish) = GATES.get().unwrap();
            start.wait();
            finish.wait();
            Ok(Self)
        }
    }
    let gates = GATES.get_or_init(|| (Barrier::new(2), Barrier::new(2)));
    let directory = tempfile::tempdir()?;
    let assets = Assets::new(directory.path())?;
    let handle = assets.load::<Blocking>(Source::Bytes {
        key: "blocking".into(),
        data: Arc::from(&b"payload"[..]),
    });
    assert_eq!(handle.status(), AssetStatus::Unloaded);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| handle.get().unwrap());
        gates.0.wait();
        assert_eq!(handle.status(), AssetStatus::Loading);
        gates.1.wait();
        let _value = task.join().unwrap();
    });
    assert_eq!(handle.status(), AssetStatus::Ready);
    Ok(())
}

#[test]
fn decoder_panics_and_payload_budget_failures_are_sticky() -> crate::Result {
    struct Panics;

    impl Asset for Panics {
        const KIND: &'static str = "panic-fixture";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            panic!("expected decoder panic")
        }
    }

    struct Oversized;

    impl Asset for Oversized {
        const KIND: &'static str = "budget-fixture";

        fn decode(_: &[u8], _: &LoadContext) -> crate::Result<Self> {
            Ok(Self)
        }

        fn retained_bytes(&self) -> usize {
            usize::MAX
        }
    }
    let directory = tempfile::tempdir()?;
    let assets = Assets::new(directory.path())?;
    let source = Source::Bytes {
        key: "fixture".into(),
        data: Arc::from(&b"bytes"[..]),
    };
    let handle = assets.load::<Panics>(source.clone());
    assert!(
        handle
            .get()
            .err()
            .unwrap()
            .to_string()
            .contains("expected decoder panic")
    );
    assert_eq!(handle.status(), AssetStatus::Failed);
    let failure = handle.error().unwrap();
    assert!(handle.get().is_err());
    assert!(Arc::ptr_eq(&failure, &handle.error().unwrap()));
    let oversized = assets.load::<Oversized>(source);
    assert_eq!(
        oversized
            .get()
            .err()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(oversized.status(), AssetStatus::Failed);
    Ok(())
}

struct Counted(Vec<u8>);

impl Asset for Counted {
    const KIND: &'static str = "test-data";

    fn decode(bytes: &[u8], _: &LoadContext) -> crate::Result<Self> {
        Ok(Self(bytes.to_vec()))
    }

    fn retained_bytes(&self) -> usize {
        self.0.len()
    }
}

#[test]
fn lazy_handles_share_one_concurrent_decode_across_owners() -> crate::Result {
    static DECODE_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Counted(Vec<u8>);

    impl Asset for Counted {
        const KIND: &'static str = "concurrent-test-data";

        fn decode(bytes: &[u8], _: &LoadContext) -> crate::Result<Self> {
            DECODE_COUNT.fetch_add(1, Ordering::SeqCst);
            Ok(Self(bytes.to_vec()))
        }

        fn retained_bytes(&self) -> usize {
            self.0.len()
        }
    }

    let directory = tempfile::tempdir()?;
    let first = Assets::new(directory.path())?;
    let second = Assets::new(directory.path())?.with_pool(first.pool().clone());
    let source = Source::Bytes {
        key: "same".into(),
        data: Arc::from(&b"shared"[..]),
    };
    let handle = first.load::<Counted>(source.clone());
    assert_eq!(handle.status(), AssetStatus::Unloaded);
    let previous = DECODE_COUNT.load(Ordering::SeqCst);
    std::thread::scope(|scope| {
        let tasks = (0..8)
            .map(|_| scope.spawn(|| second.load::<Counted>(source.clone()).get().unwrap()))
            .collect::<Vec<_>>();
        let values = tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>();
        assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
    });
    assert_eq!(DECODE_COUNT.load(Ordering::SeqCst), previous + 1);
    assert_eq!(handle.status(), AssetStatus::Ready);
    assert_eq!(handle.get()?.0, b"shared");
    let changed = first.load::<Counted>(Source::Bytes {
        key: "same".into(),
        data: Arc::from(&b"different"[..]),
    });
    assert_eq!(changed.get()?.0, b"different");
    Ok(())
}

#[test]
fn project_pools_isolate_identity_budget_and_cross_thread_diagnostics() -> crate::Result {
    let directory = tempfile::tempdir()?;
    let first = Assets::new(directory.path())?;
    let second = Assets::new(directory.path())?;
    assert!(!first.pool().same_pool(second.pool()));
    let source = Source::Bytes {
        key: "isolated".into(),
        data: Arc::from(&b"bytes"[..]),
    };
    let first_handle = first.load::<Counted>(source.clone());
    assert_eq!(first.pool().retained_bytes(), 5);
    assert_eq!(second.pool().retained_bytes(), 0);
    let second_handle = second.load::<Counted>(source);
    assert!(!Arc::ptr_eq(&first_handle.get()?, &second_handle.get()?));
    let diagnostics = RequestDiagnostics::default();
    let other_request = RequestDiagnostics::default();
    let missing = first.load::<Counted>("missing-isolated");
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                assert!(missing.get_with(&diagnostics).is_err());
            })
            .join()
            .unwrap();
    });
    assert_eq!(diagnostics.errors().len(), 1);
    assert!(other_request.errors().is_empty());
    assert!(missing.get_with(&diagnostics).is_err());
    assert_eq!(diagnostics.errors().len(), 1);
    Ok(())
}

#[test]
fn file_failures_are_lazy_sticky_classified_and_keep_original_source() -> crate::Result {
    let directory = tempfile::tempdir()?;
    let assets = Assets::new(directory.path())?;
    let handle = assets.load::<Counted>("missing.data");
    assert_eq!(handle.status(), AssetStatus::Unloaded);
    let error = handle.get().err().unwrap();
    assert_eq!(error.kind(), crate::ErrorKind::Asset);
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::NotFound
    );
    let failure = handle.error().unwrap();
    std::fs::write(directory.path().join("missing.data"), b"new")?;
    assert!(handle.get().is_err());
    assert!(Arc::ptr_eq(&failure, &handle.error().unwrap()));
    assert_eq!(handle.status(), AssetStatus::Failed);
    let invalid = assets.load::<Counted>("../escape");
    assert_eq!(invalid.status(), AssetStatus::Failed);
    assert!(invalid.error().unwrap().to_string().contains("../escape"));
    Ok(())
}
