use std::time::Instant;
use zxbs::{
    build, cache, config, directory, hash::HashMap, interner, plan, scan, timestamp::timestamp,
    tracer, thread_pool::ThreadPool,
};

fn make_duration(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap()
}

fn run() {
    let (config, subcommand) = config::Config::parse_cli();

    let config: &'static config::Config = Box::leak(Box::new(config));

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    // Use more threads then the physical cpu count because we might do IO heavy workloads in some parts of the
    // program.
    //
    // The parts of the program that uses the threadpool are responsible to use it with a concurrency limit if the
    // workloads are CPU bound.
    let thread_pool: &'static ThreadPool = Box::leak(Box::new(ThreadPool::new(num_cpus::get() * 8)));

    let start_timestamp = timestamp();

    let tracer: &'static mut tracer::Tracer =
        Box::leak(Box::new(tracer::Tracer::new(start_timestamp)));

    let mut top_level_timings: HashMap<String, (u64, u64)> = HashMap::default();

    let beginning = Instant::now();

    let mut t0 = Instant::now();
    let cache_state_load_start = make_duration(&beginning);
    let mut cache_state = cache::load(config.cache_path);
    top_level_timings.insert(
        "CacheState Load".to_owned(),
        (cache_state_load_start, make_duration(&t0)),
    );

    t0 = Instant::now();
    let scan_start = make_duration(&beginning);
    let info = scan::scan(config, interner, thread_pool);
    info.check_subcommand(interner, &subcommand);
    top_level_timings.insert("Scan".to_owned(), (scan_start, make_duration(&t0)));

    t0 = Instant::now();
    let dir_setup_start = make_duration(&beginning);
    directory::setup_build_dir(config, &info, interner, thread_pool);
    top_level_timings.insert(
        "Build Directory Setup".to_owned(),
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
        start_timestamp,
        thread_pool,
    );
    top_level_timings.insert("Plan".to_owned(), (plan_start, make_duration(&t0)));

    t0 = Instant::now();
    let build_start = make_duration(&beginning);
    build::build(
        if config.time_trace {
            Some(tracer)
        } else {
            None
        },
        interner,
        tasks,
        beginning,
        thread_pool,
    );
    top_level_timings.insert("Build".to_owned(), (build_start, make_duration(&t0)));

    t0 = Instant::now();
    let cache_state_finish = make_duration(&beginning);
    cache::finish(cache_state, config.cache_path, start_timestamp);
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
