use std::time::Instant;

use zxbs::{build, config, interner, plan, scan};

type Executor = smol::Executor<'static>;

async fn run(executor: &'static Executor) {
    let config = config::Config::parse_args();

    let _ = smol::fs::remove_dir_all(config.build_path).await;
    smol::fs::create_dir_all(config.build_path).await.unwrap();

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let start_time = Instant::now();

    let mut t0 = Instant::now();
    let info = scan::scan(&config, interner).await;
    eprintln!("scan took {}ms", t0.elapsed().as_millis());
    t0 = Instant::now();

    let tasks = plan::plan(&config, interner, &info);
    eprintln!("plan took {}ms", t0.elapsed().as_millis());
    t0 = Instant::now();

    let build = build::Build::new(executor, interner, tasks);
    build.await;
    eprintln!("build took {}ms", t0.elapsed().as_millis());

    eprintln!("total time: {}ms", start_time.elapsed().as_millis());
}

fn main() {
    let executor = Box::leak(Box::new(Executor::new()));
    smol::future::block_on(executor.run(run(executor)));
}
