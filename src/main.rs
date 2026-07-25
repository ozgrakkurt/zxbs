use std::time::Instant;

use zxbs::{build, config, interner, plan, scan};

async fn run() {
    let config = config::Config::parse_args();

    let _ = tokio::fs::remove_dir_all(config.build_path).await;
    tokio::fs::create_dir_all(config.build_path).await.unwrap();

    let interner: &'static mut interner::Interner = Box::leak(Box::new(interner::Interner::new()));

    let start_time = Instant::now();

    let mut t0 = Instant::now();
    let info = scan::scan(&config, interner).await;
    eprintln!("scan took {}ms", t0.elapsed().as_millis());
    t0 = Instant::now();

    let tasks = plan::plan(&config, interner, &info);
    eprintln!("plan took {}ms", t0.elapsed().as_millis());
    t0 = Instant::now();

    let build = build::Build::new(interner, tasks);
    build.await;
    eprintln!("build took {}ms", t0.elapsed().as_millis());

    eprintln!("total time: {}ms", start_time.elapsed().as_millis());
}

#[tokio::main]
async fn main() {
    run().await;
}
