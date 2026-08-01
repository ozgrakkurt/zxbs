use std::path::Path;

pub async fn make_baselib(src_path: &Path) {
    let path = src_path.join("base");
    tokio::fs::create_dir(&path).await.unwrap();

    let mod_path = path.join("mod");
    tokio::fs::create_dir(&mod_path).await.unwrap();

    tokio::fs::write(
        mod_path.join("base.cppm"),
        r#"
            export module base;
            export import :vec;
            export import :start;
            export import :arith;
            export import :alloc;
        "#,
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("vec.cppm"),
        r#"
            export module base:vec;
            import :alloc;
            import :arith;
        "#,
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("start.cppm"),
        r#"
            export module base:start;
        "#,
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("arith.cppm"),
        r#"
            export module base:arith;
            import :start;

            export namespace base {
                int add(int a, int b);
            }
        "#,
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("alloc.cppm"),
        r#"
            export module base:alloc;
            import :start;
            import :arith;
        "#,
    )
    .await
    .unwrap();

    let lib_path = path.join("lib");
    tokio::fs::create_dir(&lib_path).await.unwrap();

    tokio::fs::write(
        lib_path.join("memcmp.cc"),
        r#"
            import base;

            __asm__(".text\n"
            ".type _start, @function\n"
            ".global _start\n"
            "_start:\n\t"
            "xorq %rbp, %rbp\n\t"
            "popq %rdi\n\t"
            "movq %rsp, %rsi\n\t"
            "andq $-16, %rsp\n\t"
            "callq main\n\t"
            "movq %rax, %rdi\n\t"
            "movq $60, %rax\n\t"
            "syscall\n\t");
        "#,
    )
    .await
    .unwrap();

    tokio::fs::write(
        lib_path.join("memcpy.cc"),
        r#"
            module base;

            namespace base {
                int add(int a, int b) {
                    return a + b;
                }
            }
        "#,
    )
    .await
    .unwrap();

    let bin_path = path.join("bin");
    tokio::fs::create_dir(&bin_path).await.unwrap();

    tokio::fs::write(
        bin_path.join("memcpy_bench.cc"),
        r#"
            import base;

            int main(void) {
                int res = base::add(1, 2);
                if (res != 3) {
                    __builtin_trap();
                }
                return 0;
            }
        "#,
    )
    .await
    .unwrap();

    let test_path = path.join("test");
    tokio::fs::create_dir(&test_path).await.unwrap();

    for name in ["memcmp.cc", "vec.cc", "arith.cc"] {
        tokio::fs::write(
            test_path.join(name),
            r#"
                import base;

                int main(void) {
                    int res = base::add(1, 2);
                    if (res != 3) {
                        __builtin_trap();
                    }
                    return 0;
                }
            "#,
        )
        .await
        .unwrap();
    }

    tokio::fs::write(path.join("deps"), "").await.unwrap();
}

pub async fn make_iolib(src_path: &Path) {
    let path = src_path.join("io");
    tokio::fs::create_dir(&path).await.unwrap();

    let mod_path = path.join("mod");
    tokio::fs::create_dir(&mod_path).await.unwrap();

    tokio::fs::write(
        mod_path.join("io.cppm"),
        "
            export module io;
            export import :executor;
            export import :fs;
            export import :net;
            export import :base;
        ",
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("executor.cppm"),
        "
            export module io:executor;
        ",
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("fs.cppm"),
        "
            export module io:fs;
            import :executor;
        ",
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("net.cppm"),
        "
            export module io:net;
            import :executor;
            import base;
        ",
    )
    .await
    .unwrap();

    tokio::fs::write(
        mod_path.join("base.cppm"),
        "
            export module io:base;
        ",
    )
    .await
    .unwrap();

    let lib_path = path.join("lib");
    tokio::fs::create_dir(&lib_path).await.unwrap();

    let bin_path = path.join("bin");
    tokio::fs::create_dir(&bin_path).await.unwrap();

    tokio::fs::write(
        bin_path.join("direct_io_example.cc"),
        r#"
            import io;
            import base;

            int main(void) {
                int res = base::add(1, 2);
                if (res != 3) {
                    __builtin_trap();
                }
                return 0;
            }
        "#,
    )
    .await
    .unwrap();

    let test_path = path.join("test");
    tokio::fs::create_dir(&test_path).await.unwrap();

    for name in ["executor_ordering.cc", "fs.cc", "net.cc"] {
        tokio::fs::write(
            test_path.join(name),
            r#"
                import base;
                import io;

                int main(void) {
                    int res = base::add(1, 2);
                    if (res != 3) {
                        __builtin_trap();
                    }
                    return 0;
                }
            "#,
        )
        .await
        .unwrap();
    }

    tokio::fs::write(path.join("deps"), "base").await.unwrap();
}

pub async fn make_myapp(src_path: &Path) {
    let path = src_path.join("myapp");
    tokio::fs::create_dir(&path).await.unwrap();

    let mod_path = path.join("mod");
    tokio::fs::create_dir(&mod_path).await.unwrap();

    tokio::fs::write(
        mod_path.join("myapp.cppm"),
        "
            export module myapp;
            import base;
            import io;
        ",
    )
    .await
    .unwrap();

    let lib_path = path.join("lib");
    tokio::fs::create_dir(&lib_path).await.unwrap();

    let bin_path = path.join("bin");
    tokio::fs::create_dir(&bin_path).await.unwrap();

    tokio::fs::write(
        bin_path.join("myapp.cc"),
        r#"
            import myapp;
            import io;
            import base;

            int main(void) {
                int res = base::add(1, 2);
                if (res != 3) {
                    __builtin_trap();
                }
                return 0;
            }
        "#,
    )
    .await
    .unwrap();

    let test_path = path.join("test");
    tokio::fs::create_dir(&test_path).await.unwrap();

    tokio::fs::write(
        path.join("deps"),
        r#"
            base
            io
        "#,
    )
    .await
    .unwrap();
}
