use std::time::Instant;
use zxbs::{
    base_hash::calculate_base_hash, build, cache, config, directory, hash::HashMap, interner, plan,
    scan, thread_pool::ThreadPool, timestamp::timestamp, tracer,
};

fn make_duration(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap()
}

fn run() {
    let (config, subcommand) = config::Config::parse_cli();

    let config: &'static config::Config = Box::leak(Box::new(config));

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    // It is ok to use more threads than the physical cpu count because the program does IO heavy
    // workloads in some parts.
    let thread_pool = ThreadPool::new(128);

    let start_timestamp = timestamp();

    let tracer: &'static mut tracer::Tracer =
        Box::leak(Box::new(tracer::Tracer::new(start_timestamp)));

    let mut top_level_timings: HashMap<String, (u64, u64)> = HashMap::default();

    let beginning = Instant::now();

    let mut t0 = Instant::now();
    let cache_state_load_start = make_duration(&beginning);
    let mut cache_state = cache::load(config.cache_path, &thread_pool, start_timestamp);
    top_level_timings.insert(
        "CacheState Load".to_owned(),
        (cache_state_load_start, make_duration(&t0)),
    );

    t0 = Instant::now();
    let base_hash_start = make_duration(&beginning);
    // The base hash incorporates the build configuration and the hash of this
    // binary. It is reused by both the scan (for the clang-scan-deps cache key)
    // and plan (for the deep hashes).
    let base_hash = calculate_base_hash(config, &thread_pool);
    top_level_timings.insert(
        "Base Hash".to_owned(),
        (base_hash_start, make_duration(&t0)),
    );

    t0 = Instant::now();
    let scan_start = make_duration(&beginning);
    let info = scan::scan(
        config,
        interner,
        &thread_pool,
        base_hash,
        &mut cache_state,
        start_timestamp,
    );
    info.check_subcommand(interner, &subcommand);
    top_level_timings.insert("Scan".to_owned(), (scan_start, make_duration(&t0)));

    t0 = Instant::now();
    let dir_setup_start = make_duration(&beginning);
    directory::setup_build_directories(config, &info, interner, &thread_pool);
    top_level_timings.insert(
        "Directory Setup".to_owned(),
        (dir_setup_start, make_duration(&t0)),
    );

    t0 = Instant::now();
    let plan_start = make_duration(&beginning);
    let tasks = plan::plan(
        config,
        &subcommand,
        interner,
        &info,
        &mut cache_state,
        base_hash,
        start_timestamp,
    );
    top_level_timings.insert("Plan".to_owned(), (plan_start, make_duration(&t0)));

    t0 = Instant::now();
    let build_start = make_duration(&beginning);
    build::Build::new(
        if config.time_trace {
            Some(tracer)
        } else {
            None
        },
        interner,
        tasks,
        beginning,
        &thread_pool,
    )
    .build();
    top_level_timings.insert("Build".to_owned(), (build_start, make_duration(&t0)));

    t0 = Instant::now();
    let cache_state_finish = make_duration(&beginning);
    cache::finish(
        cache_state,
        config.cache_path,
        &thread_pool,
        start_timestamp,
    );
    top_level_timings.insert(
        "CacheState Finish".to_owned(),
        (cache_state_finish, make_duration(&t0)),
    );

    if config.time_trace {
        let json = tracer.print_json(interner, &top_level_timings);
        std::fs::write(config.build_path.join("time_trace.json"), json).unwrap();
    }
}

fn main() {
    run();
}
