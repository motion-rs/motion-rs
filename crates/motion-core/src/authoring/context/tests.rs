//! Unit tests for scene contexts, scene caches and rendering-cache partitions.

use super::*;
use crate::Shape;
use crate::{TimeLiteral, Tween};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn render_cache_shares_bounded_partitions_without_cross_domain_collisions() {
    let cache = RenderCache::with_limits(CacheLimits {
        entries: 1,
        bytes: 128,
    });
    let shared = cache.clone();
    let get = |cache: &RenderCache, partition, key| {
        cache
            .partition(partition)
            .get_with_cost(key, || vec![key; 8], |value| value.len() * 8)
    };
    let first = get(&cache, CachePartition::Paragraph, 7_u64);
    assert!(Arc::ptr_eq(
        &first,
        &get(&shared, CachePartition::Paragraph, 7)
    ));
    assert!(!Arc::ptr_eq(
        &first,
        &get(&shared, CachePartition::Glyph, 7)
    ));
    let _ = get(&shared, CachePartition::Paragraph, 8);
    assert!(!Arc::ptr_eq(
        &first,
        &get(&cache, CachePartition::Paragraph, 7)
    ));
    assert!(cache.partition(CachePartition::Paragraph).retained_bytes() <= 128);
    shared.clear();
    assert!(cache.partition(CachePartition::Paragraph).is_empty());
    assert_eq!(first.as_slice(), &[7; 8]);
    let partition = cache.partition(CachePartition::Coverage);
    assert!(partition.lookup::<_, Vec<u64>>(&9_u64).is_none());
    let stored = partition.insert_with_cost(9_u64, vec![1_u64; 8], |_| 64);
    assert!(Arc::ptr_eq(&stored, &partition.lookup(&9_u64).unwrap()));
}

#[derive(PartialEq, Eq)]
struct CollidingKey(u8);

impl std::hash::Hash for CollidingKey {
    fn hash<Hasher: std::hash::Hasher>(&self, state: &mut Hasher) {
        0_u8.hash(state);
    }
}

#[test]
fn scene_cache_distinguishes_unequal_keys_with_identical_hashes() {
    for accounted in [false, true] {
        let cache = SceneCache::with_limits(CacheLimits {
            entries: 2,
            bytes: 64,
        });
        let context = context(&cache, Time::ZERO);
        let fetch = |key| {
            if accounted {
                context.cache_with_cost(CollidingKey(key), || u32::from(key), |_| 4)
            } else {
                context.cache(CollidingKey(key), || u32::from(key))
            }
        };
        let first = fetch(1);
        let second = fetch(2);
        assert_eq!((*first, *second), (1, 2));
        assert_eq!(cache.len(), 2);
        assert!(Arc::ptr_eq(&first, &fetch(1)));
        assert!(Arc::ptr_eq(&second, &fetch(2)));
        let charge = std::mem::size_of::<CollidingKey>() + std::mem::size_of::<u32>();
        assert_eq!(
            cache.retained_bytes(),
            if accounted { 2 * charge } else { 0 }
        );
        assert_eq!(*fetch(3), 3);
        assert!(Arc::ptr_eq(&second, &fetch(2)));
        assert!(!Arc::ptr_eq(&first, &fetch(1)));
        assert_eq!(cache.len(), 2);
        assert_eq!((*first, *second), (1, 2));
    }
}

fn cached_typed_key<Key: Eq + Hash + Send + Sync + 'static>(
    context: &SceneContext<'_>,
    key: Key,
    value: u32,
    accounted: bool,
) -> Arc<u32> {
    if accounted {
        context.cache_with_cost(key, || value, |_| 4)
    } else {
        context.cache(key, || value)
    }
}

#[test]
fn scene_cache_distinguishes_key_types_with_identical_hashes() {
    for accounted in [false, true] {
        let cache = SceneCache::with_limits(CacheLimits {
            entries: 2,
            bytes: 64,
        });
        let context = context(&cache, Time::ZERO);
        let first = cached_typed_key(&context, CollidingKey(1), 11, accounted);
        let second = cached_typed_key(&context, 0_u8, 22, accounted);
        assert_eq!((*first, *second), (11, 22));
        assert_eq!(cache.len(), 2);
        assert!(Arc::ptr_eq(
            &first,
            &cached_typed_key(&context, CollidingKey(1), 99, accounted)
        ));
        assert!(Arc::ptr_eq(
            &second,
            &cached_typed_key(&context, 0_u8, 99, accounted)
        ));
    }
}

#[test]
fn scene_cache_rechecks_full_keys_after_concurrent_misses() {
    for accounted in [false, true] {
        for same_key in [false, true] {
            let cache = SceneCache::with_limits(CacheLimits {
                entries: 2,
                bytes: 64,
            });
            let context = context(&cache, Time::ZERO);
            let barrier = std::sync::Barrier::new(2);
            let second_key = if same_key { 1 } else { 2 };
            let fetch = |key| {
                let build = || {
                    barrier.wait();
                    u32::from(key)
                };
                if accounted {
                    context.cache_with_cost(CollidingKey(key), build, |_| 4)
                } else {
                    context.cache(CollidingKey(key), build)
                }
            };
            let (first, second) = std::thread::scope(|scope| {
                let first = scope.spawn(|| fetch(1));
                let second = scope.spawn(|| fetch(second_key));
                (first.join().unwrap(), second.join().unwrap())
            });
            assert_eq!((*first, *second), (1, u32::from(second_key)));
            assert_eq!(Arc::ptr_eq(&first, &second), same_key);
            let count = if same_key { 1 } else { 2 };
            assert_eq!(cache.len(), count);
            let charge = std::mem::size_of::<CollidingKey>() + std::mem::size_of::<u32>();
            assert_eq!(
                cache.retained_bytes(),
                if accounted { charge * count } else { 0 }
            );
        }
    }
}

#[test]
fn scene_cache_cost_includes_retained_key_inline_storage() {
    for limit in [32, 33] {
        let cache = SceneCache::with_limits(CacheLimits {
            entries: 2,
            bytes: limit,
        });
        let context = context(&cache, Time::ZERO);
        let fetch = || context.cache_with_cost([0_u8; 32], || 7_u8, |_| 1);
        let first = fetch();
        let second = fetch();
        assert_eq!((*first, *second), (7, 7));
        assert_eq!(Arc::ptr_eq(&first, &second), limit == 33);
        assert_eq!(cache.len(), usize::from(limit == 33));
        assert_eq!(cache.retained_bytes(), if limit == 33 { 33 } else { 0 });
    }
}

#[test]
fn ordinary_cache_is_entry_bounded_and_only_explicit_costs_charge_bytes() {
    use super::*;
    let cache = SceneCache::with_limits(CacheLimits {
        entries: 2,
        bytes: 1,
    });
    let fetch = || cache.get("large", || vec![1_u8; 4096]);
    assert!(Arc::ptr_eq(&fetch(), &fetch()));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.retained_bytes(), 0);
    let costed = || cache.get_with_cost("charged", || vec![2_u8; 4096], |value| value.len());
    assert!(!Arc::ptr_eq(&costed(), &costed()));
    assert_eq!(cache.len(), 1);
}

#[test]
fn checked_context_inputs_and_declared_cache_bytes_preserve_ownership() {
    let cache = SceneCache::with_limits(CacheLimits {
        entries: 8,
        bytes: 96,
    });
    assert!(
        SceneContext::try_new(Time::ZERO, FrameRate::from(30), Size::new(0.0, 1.0), &cache)
            .is_err()
    );
    let context =
        SceneContext::try_new(Time::ZERO, FrameRate::from(30), Size::new(1.0, 1.0), &cache)
            .unwrap();
    assert!(context.try_with_clip_duration((-1).seconds()).is_err());
    assert_eq!(
        context.try_shifted(1.seconds()).unwrap().time(),
        (-1).seconds().after_origin()
    );
    let extreme = SceneContext::new(
        Time::from_ticks(i64::MIN),
        FrameRate::from(30),
        Size::new(1.0, 1.0),
        &cache,
    );
    assert!(extreme.try_shifted(Duration::from_ticks(1)).is_err());
    let get = |key: u8, count: usize| {
        context.cache_with_cost(
            key,
            || vec![key; count],
            |value| std::mem::size_of::<Vec<u8>>() + value.len(),
        )
    };
    let pinned = get(1, 48);
    let retained = get(2, 48);
    assert_eq!(cache.len(), 1);
    assert!(cache.retained_bytes() <= cache.max_bytes());
    assert!(Arc::ptr_eq(&retained, &get(2, 48)));
    assert_eq!(pinned.as_slice(), &[1; 48]);
    assert_eq!(get(3, 256).len(), 256);
    assert_eq!(cache.len(), 1);
    cache.clear();
    assert_eq!(cache.retained_bytes(), 0);
    assert_eq!(retained.len(), 48);
}

fn layout_node(bounds: Rect) -> Node {
    Node::from(bounds.fill(crate::Color::WHITE))
}

#[test]
fn measure_or_falls_back_only_when_measurement_fails() {
    let cache = SceneCache::new(0);
    let context = context(&cache, Time::ZERO);
    let valid = layout_node(Rect::new(1.0, 2.0, 5.0, 6.0));
    assert_eq!(
        context.measure_or(&valid, Rect::default()),
        Rect::new(1.0, 2.0, 5.0, 6.0)
    );
    let invalid = Node::from(Rect::new(0.0, 0.0, f64::NAN, 1.0).fill(crate::Color::WHITE));
    assert!(context.measure(&invalid).is_err());
    let fallback = Rect::new(0.0, 0.0, 9.0, 9.0);
    assert_eq!(context.measure_or(&invalid, fallback), fallback);
}

#[test]
fn relative_layout_places_each_side_and_aligns_only_the_requested_axis() {
    use crate::authoring::Side;
    let cache = SceneCache::new(0);
    let context = context(&cache, Time::ZERO);
    let subject = layout_node(Rect::new(5.0, 7.0, 25.0, 17.0));
    let reference = layout_node(Rect::new(100.0, 200.0, 160.0, 240.0));
    for (side, expected) in [
        (Side::Left, Rect::new(77.0, 215.0, 97.0, 225.0)),
        (Side::Right, Rect::new(163.0, 215.0, 183.0, 225.0)),
        (Side::Top, Rect::new(120.0, 187.0, 140.0, 197.0)),
        (Side::Bottom, Rect::new(120.0, 243.0, 140.0, 253.0)),
    ] {
        let placed = context
            .try_next_to(subject.clone(), &reference, side, 3.0)
            .unwrap();
        assert_eq!(context.measure(&placed).unwrap(), expected);
    }
    for (side, expected) in [
        (Side::Left, Rect::new(100.0, 7.0, 120.0, 17.0)),
        (Side::Right, Rect::new(140.0, 7.0, 160.0, 17.0)),
        (Side::Top, Rect::new(5.0, 200.0, 25.0, 210.0)),
        (Side::Bottom, Rect::new(5.0, 230.0, 25.0, 240.0)),
    ] {
        let aligned = context
            .try_align_to(subject.clone(), &reference, side)
            .unwrap();
        assert_eq!(context.measure(&aligned).unwrap(), expected);
    }
    assert_eq!(
        context.measure(&subject).unwrap(),
        Rect::new(5.0, 7.0, 25.0, 17.0)
    );
    let overlap = context
        .try_next_to(subject, &reference, Side::Right, -2.0)
        .unwrap();
    assert_eq!(context.measure(&overlap).unwrap().left, 158.0);
}

#[test]
fn relative_layout_preserves_existing_spatial_properties_and_rejects_invalid_geometry() {
    use crate::authoring::Side;
    use crate::{NodeModifiers, Pivot};
    let cache = SceneCache::new(0);
    let context = context(&cache, Time::ZERO);
    let subject = layout_node(Rect::new(0.0, 0.0, 20.0, 10.0))
        .pivot(Pivot::TopLeft)
        .scale(2.0)
        .rotate(std::f64::consts::FRAC_PI_2)
        .at(Point::new(40.0, 50.0));
    let reference = layout_node(Rect::new(100.0, 100.0, 140.0, 120.0));
    let placed = context
        .try_next_to(subject.clone(), &reference, Side::Bottom, 4.0)
        .unwrap();
    assert_eq!(
        context.measure(&placed).unwrap(),
        Rect::new(110.0, 124.0, 130.0, 164.0)
    );
    assert_eq!(
        context.measure(&subject).unwrap(),
        Rect::new(20.0, 50.0, 40.0, 90.0)
    );
    for gap in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            context
                .try_next_to(subject.clone(), &reference, Side::Left, gap)
                .unwrap_err()
                .field,
            "gap"
        );
    }
    let unknown = layout_node(Rect::new(0.0, 0.0, f64::NAN, 1.0));
    assert_eq!(
        context
            .try_align_to(unknown, &reference, Side::Left)
            .unwrap_err()
            .kind,
        crate::ValidationErrorKind::InvalidValue
    );
    let left = layout_node(Rect::new(-f64::MAX, 0.0, -f64::MAX, 1.0));
    let right = layout_node(Rect::new(f64::MAX, 0.0, f64::MAX, 1.0));
    assert_eq!(
        context
            .try_align_to(left, &right, Side::Left)
            .unwrap_err()
            .field,
        "translation"
    );
}

#[test]
fn relative_layout_is_independent_of_seek_order_and_checks_both_inputs() {
    use crate::authoring::Side;
    let cache = SceneCache::new(0);
    let root = context(&cache, Time::ZERO);
    let subject = layout_node(Rect::new(0.0, 0.0, 20.0, 10.0));
    let reference = layout_node(Rect::new(100.0, 200.0, 160.0, 240.0));
    for ticks in [100, -999, 0, 100] {
        let context = root.shifted(Duration::from_ticks(ticks));
        let placed = context
            .try_next_to(subject.clone(), &reference, Side::Right, 3.0)
            .unwrap();
        assert_eq!(
            context.measure(&placed).unwrap(),
            Rect::new(163.0, 215.0, 183.0, 225.0)
        );
    }
    let unknown = layout_node(Rect::new(0.0, 0.0, f64::NAN, 1.0));
    assert_eq!(
        root.try_next_to(subject, &unknown, Side::Bottom, 0.0)
            .unwrap_err()
            .kind,
        crate::ValidationErrorKind::InvalidValue
    );
}

#[test]
fn node_layout_measurement_is_independent_of_context_time() {
    use crate::NodeModifiers;
    let cache = SceneCache::new(0);
    let node = layout_node(Rect::new(0.0, 0.0, 20.0, 10.0))
        .at(Point::new(60.0, 40.0))
        .translate(-0.25, 1.5);
    for time in [(-5).seconds(), Duration::ZERO, 100.seconds()] {
        assert_eq!(
            context(&cache, time.after_origin()).measure(&node).unwrap(),
            Rect::new(49.75, 36.5, 69.75, 46.5)
        );
    }
}

fn context(cache: &SceneCache, time: Time) -> SceneContext<'_> {
    SceneContext::new(time, FrameRate::from(30), Size::new(640.5, 360.25), cache)
}

#[test]
fn local_shifts_preserve_global_inputs_and_sample_tick_animations() {
    let cache = SceneCache::default();
    let root = context(&cache, 3.seconds().after_origin()).with_clip_duration(4.seconds());
    let local = root.shifted(1.seconds()).shifted(500.milliseconds());
    assert_eq!(local.time(), 1500.milliseconds().after_origin());
    assert_eq!(local.global_time(), 3.seconds().after_origin());
    assert_eq!(local.seconds(), 1.5);
    assert_eq!(local.clip_duration(), Some(4.seconds()));
    assert_eq!(local.progress(), 0.375);
    assert_eq!(local.sample(&Tween::new(0.0, 10.0, 3.seconds())), 5.0);
    let signal: Arc<dyn Signal<Duration>> = Arc::new(|time| time);
    assert_eq!(local.sample(&signal), local.elapsed());
    assert_eq!(local.size(), Size::new(640.5, 360.25));
    assert_eq!(local.bounds(), Rect::new(0.0, 0.0, 640.5, 360.25));
    assert_eq!(local.center(), Point::new(320.25, 180.125));
    assert_eq!(
        root.shifted((-1).seconds()).time(),
        4.seconds().after_origin()
    );
    assert_eq!(root.time(), 3.seconds().after_origin());
}

#[test]
fn progress_uses_signed_integer_boundaries_including_extreme_queries() {
    let cache = SceneCache::new(0);
    let start = Time::from_ticks((1_i64 << 53) + 1);
    for (ticks, expected) in [
        (start.ticks() - 1, 0.0),
        (start.ticks(), 0.0),
        (start.ticks() + 1, 0.5),
        (start.ticks() + 2, 1.0),
    ] {
        assert_eq!(
            context(&cache, Time::from_ticks(ticks)).progress_in(start, Duration::from_ticks(2)),
            expected
        );
    }
    assert_eq!(
        context(&cache, Time::from_ticks(i64::MAX))
            .progress_in(Time::from_ticks(i64::MIN), Duration::from_ticks(i64::MAX)),
        1.0
    );
    assert_eq!(
        context(&cache, Time::from_ticks(i64::MIN))
            .progress_in(Time::from_ticks(i64::MAX), 1.seconds()),
        0.0
    );
    for (time, expected) in [
        (Time::from_ticks(-1), 0.0),
        (Time::ZERO, 1.0),
        (1.seconds().after_origin(), 1.0),
    ] {
        assert_eq!(
            context(&cache, time)
                .with_clip_duration(Duration::ZERO)
                .progress(),
            expected
        );
    }
    assert_eq!(context(&cache, 10.seconds().after_origin()).progress(), 0.0);
}

#[test]
fn frame_intervals_and_invalid_inputs_keep_checked_time_policy() {
    let cache = SceneCache::new(0);
    let rate = FrameRate::new(30_000, 1001);
    let context = || SceneContext::new(Time::ZERO, rate, Size::new(1.0, 1.0), &cache);
    assert_eq!(context().frame_rate(), rate);
    assert_eq!(context().frames(30_000), 1001.seconds());
    assert_eq!(context().frames(-30_000), (-1001).seconds());
    assert!(
        std::panic::catch_unwind(|| context().with_clip_duration(Duration::from_ticks(-1)))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| context().progress_in(Time::ZERO, Duration::from_ticks(-1)))
            .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| context().shifted(Duration::from_ticks(i64::MIN))).is_err()
    );
    for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            std::panic::catch_unwind(|| SceneContext::new(
                Time::ZERO,
                rate,
                Size::new(width, 1.0),
                &cache
            ))
            .is_err()
        );
    }
}

fn cached(context: &SceneContext<'_>, key: u32, builds: &AtomicUsize) -> Arc<u32> {
    context.cache(key, || {
        builds.fetch_add(1, Ordering::SeqCst);
        key * key
    })
}

fn typed<Value: Send + Sync + 'static>(
    context: &SceneContext<'_>,
    build: impl FnOnce() -> Value,
) -> Arc<Value> {
    context.cache("same-key", build)
}

#[test]
fn cache_is_typed_callsite_scoped_and_lru_bounded() {
    let cache = SceneCache::new(2);
    let context = context(&cache, Time::ZERO);
    let builds = AtomicUsize::new(0);
    let first = cached(&context, 1, &builds);
    cached(&context, 2, &builds);
    assert!(Arc::ptr_eq(&first, &cached(&context, 1, &builds)));
    cached(&context, 3, &builds);
    assert_eq!(builds.load(Ordering::SeqCst), 3);
    cached(&context, 2, &builds);
    assert_eq!(builds.load(Ordering::SeqCst), 4);
    assert_eq!(cache.len(), cache.capacity());
    assert_eq!(*first, 1);
    cache.clear();
    assert!(cache.is_empty());
    let first = context.cache("same", || 11_u32);
    let second = context.cache("same", || 22_u32);
    assert_eq!((*first, *second), (11, 22));
    cache.clear();
    assert_eq!(*typed(&context, || 31_u32), 31);
    assert_eq!(*typed(&context, || String::from("typed")), "typed");
    assert_eq!(cache.len(), 2);
}

#[test]
fn cache_eviction_disable_and_seek_order_do_not_change_values() {
    for capacity in [0, 1, 8] {
        let cache = SceneCache::new(capacity);
        let builds = AtomicUsize::new(0);
        for frame in [9, 0, 4, 9, 2, 0] {
            let context = context(&cache, Time::from_frame(frame, FrameRate::from(30)));
            assert_eq!(
                *cached(&context, frame as u32, &builds),
                (frame * frame) as u32
            );
            assert!(cache.len() <= capacity);
        }
        if capacity == 0 {
            assert_eq!(builds.load(Ordering::SeqCst), 6);
        }
    }
}

#[test]
fn builders_can_nest_panic_and_share_results_across_threads() {
    let cache = SceneCache::new(8);
    let context = context(&cache, Time::ZERO);
    assert_eq!(
        *context.cache("outer", || *context.cache("inner", || 12_u32)),
        12
    );
    assert!(
        std::panic::catch_unwind(|| {
            self::context(&cache, Time::ZERO).cache("panic", || -> u32 { panic!("builder") })
        })
        .is_err()
    );
    assert_eq!(*context.cache("after", || 42_u32), 42);
    let builds = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| cached(&context, 7, &builds)))
            .collect();
        let values: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
        assert_eq!(*values[0], 49);
    });
    assert!(cache.len() <= cache.capacity());
}

#[test]
fn keyed_randomness_is_identical_at_any_local_global_time_or_cache_capacity() {
    let expected = Random::from_key(("position", 17_u64)).next_float();
    for capacity in [0, 1, 16] {
        let cache = SceneCache::new(capacity);
        for ticks in [i64::MAX, 0, i64::MIN + 1, 782, -91, 0] {
            let root = context(&cache, Time::from_ticks(ticks));
            let local = root.shifted(Duration::from_ticks(1));
            assert_eq!(root.random(("position", 17_u64)), expected);
            assert_eq!(local.random(("position", 17_u64)), expected);
            let mut first = local.random_generator("stream");
            let mut second = root.random_generator("stream");
            for _ in 0..8 {
                assert_eq!(first.next_float(), second.next_float());
            }
        }
    }
}

#[test]
fn eviction_and_clear_drop_user_values_outside_the_cache_lock() {
    struct Probe {
        cache: std::sync::Weak<SceneCache>,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            let cache = self.cache.upgrade().unwrap();
            assert!(cache.entries.try_lock().is_ok());
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    let cache = Arc::new(SceneCache::new(1));
    let context = context(&cache, Time::ZERO);
    let drops = Arc::new(AtomicUsize::new(0));
    for key in [1, 2] {
        drop(context.cache(key, || Probe {
            cache: Arc::downgrade(&cache),
            drops: drops.clone(),
        }));
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    cache.clear();
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn during_is_half_open_and_nested_inputs_preserve_global_time_and_environment() {
    let cache = SceneCache::default();
    let mut calls = Vec::new();
    for ticks in [14, 9, 10, 13, 14, 10] {
        let root = context(&cache, Time::from_ticks(ticks)).with_clip_duration(100.seconds());
        root.during(Time::from_ticks(10), Duration::from_ticks(4), |local| {
            assert_eq!(local.global_time(), root.time());
            assert_eq!(local.clip_duration(), Some(Duration::from_ticks(4)));
            assert_eq!(local.size(), root.size());
            assert_eq!(local.frame_rate(), root.frame_rate());
            assert!(std::ptr::eq(local.cache, root.cache));
            calls.push(local.time().ticks());
            local.during(Time::from_ticks(2), Duration::from_ticks(2), |nested| {
                assert_eq!(nested.time(), Time::from_ticks(1));
                assert_eq!(nested.global_time(), root.time());
                assert_eq!(nested.clip_duration(), Some(Duration::from_ticks(2)));
                Node::empty()
            })
        });
    }
    assert_eq!(calls, vec![0, 3, 0]);
    context(&cache, Time::ZERO).during(Time::ZERO, Duration::ZERO, |_| panic!("empty interval"));
    let near_max = context(&cache, Time::from_ticks(i64::MAX));
    near_max.during(
        Time::from_ticks(i64::MAX - 1),
        Duration::from_ticks(4),
        |local| {
            assert_eq!(local.time(), Time::from_ticks(1));
            Node::empty()
        },
    );
    context(&cache, Time::from_ticks(i64::MIN)).during(
        Time::from_ticks(i64::MAX),
        1.seconds(),
        |_| panic!("outside"),
    );
    assert!(
        std::panic::catch_unwind(|| context(&cache, Time::ZERO).during(
            Time::ZERO,
            Duration::from_ticks(-1),
            |_| Node::empty()
        ))
        .is_err()
    );
}

#[test]
fn embed_uses_child_duration_and_clears_metadata_for_infinite_children() {
    use crate::authoring::Scene as _;
    let cache = SceneCache::default();
    let calls = AtomicUsize::new(0);
    let finite = (|local: &SceneContext<'_>| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(local.clip_duration(), Some(2.seconds()));
        assert_eq!(local.global_time(), local.time() + 1.seconds());
        Node::empty()
    })
    .with_duration(2.seconds());
    for seconds in [3, 0, 1, 2, 3, 1] {
        context(&cache, seconds.seconds().after_origin())
            .embed(&finite, 1.seconds().after_origin());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let infinite = |local: &SceneContext<'_>| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(local.time(), 1.seconds().after_origin());
        assert_eq!(local.global_time(), 2.seconds().after_origin());
        assert_eq!(local.clip_duration(), None);
        Node::empty()
    };
    context(&cache, Time::ZERO).embed(&infinite, 1.seconds().after_origin());
    context(&cache, 2.seconds().after_origin())
        .with_clip_duration(7.seconds())
        .embed(&infinite, 1.seconds().after_origin());
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let zero = (|_: &SceneContext<'_>| panic!("empty child")).with_duration(Duration::ZERO);
    context(&cache, Time::ZERO).embed(&zero, Time::ZERO);
}
