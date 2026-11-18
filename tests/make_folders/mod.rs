use std::path::Path;

pub fn make_baselib(src_path: &Path) {
    let path = src_path.join("base");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("base.cppm"),
        r#"
            export module base;
            export import :vec;
            export import :start;
            export import :arith;
            export import :alloc;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("vec.cppm"),
        r#"
            export module base:vec;
            import :alloc;
            import :arith;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("start.cppm"),
        r#"
            export module base:start;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("arith.cppm"),
        r#"
            export module base:arith;
            import :start;

            export namespace base {
                int add(int a, int b);
            }
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("alloc.cppm"),
        r#"
            export module base:alloc;
            import :start;
            import :arith;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    std::fs::write(
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
    .unwrap();

    std::fs::write(
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
    .unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
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
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    for name in ["memcmp.cc", "vec.cc", "arith.cc"] {
        std::fs::write(
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
        .unwrap();
    }

    std::fs::write(path.join("deps"), "").unwrap();
}

pub fn make_iolib(src_path: &Path) {
    let path = src_path.join("io");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("io.cppm"),
        "
            export module io;
            export import :executor;
            export import :fs;
            export import :net;
            export import :base;
        ",
    )
    .unwrap();

    std::fs::write(
        mod_path.join("executor.cppm"),
        "
            export module io:executor;
        ",
    )
    .unwrap();

    std::fs::write(
        mod_path.join("fs.cppm"),
        "
            export module io:fs;
            import :executor;
        ",
    )
    .unwrap();

    std::fs::write(
        mod_path.join("net.cppm"),
        "
            export module io:net;
            import :executor;
            import base;
        ",
    )
    .unwrap();

    std::fs::write(
        mod_path.join("base.cppm"),
        "
            export module io:base;
        ",
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
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
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    for name in ["executor_ordering.cc", "fs.cc", "net.cc"] {
        std::fs::write(
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
        .unwrap();
    }

    std::fs::write(path.join("deps"), "base").unwrap();
}

// A single entry with a diamond shaped module dependency graph: both `left`
// and `right` depend on `common`, so it is reachable from the root module
// through two different paths. (The root still has to import every submodule
// directly, which the scanner requires.) Also has multiple bins and multiple
// tests, which the other trees don't have.
//
// Not every test binary that includes this module uses every tree, so dead
// code is allowed on the fixtures below.
#[allow(dead_code)]
pub fn make_diamond(src_path: &Path) {
    let path = src_path.join("diamond");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("diamond.cppm"),
        r#"
            export module diamond;
            export import :left;
            export import :right;
            export import :common;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("left.cppm"),
        r#"
            export module diamond:left;
            import :common;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("right.cppm"),
        r#"
            export module diamond:right;
            import :common;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("common.cppm"),
        r#"
            export module diamond:common;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    std::fs::write(
        lib_path.join("helper.cc"),
        r#"
            import diamond;
        "#,
    )
    .unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    for name in ["tool_a.cc", "tool_b.cc"] {
        std::fs::write(
            bin_path.join(name),
            r#"
                import diamond;

                int main(void) {
                    return 0;
                }
            "#,
        )
        .unwrap();
    }

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    for name in ["left.cc", "right.cc"] {
        std::fs::write(
            test_path.join(name),
            r#"
                import diamond;

                int main(void) {
                    return 0;
                }
            "#,
        )
        .unwrap();
    }

    std::fs::write(path.join("deps"), "").unwrap();
}

// Two entries with a single entry level dependency: `app` depends on `util`.
// Both the root module and a submodule of `app` import the root module of
// `util`. `app` has no lib or bin parts, only a test.
#[allow(dead_code)]
pub fn make_util_app(src_path: &Path) {
    let path = src_path.join("util");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("util.cppm"),
        r#"
            export module util;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    std::fs::write(
        lib_path.join("string.cc"),
        r#"
            import util;
        "#,
    )
    .unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
        bin_path.join("inspect.cc"),
        r#"
            import util;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        test_path.join("string.cc"),
        r#"
            import util;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    std::fs::write(path.join("deps"), "").unwrap();

    let path = src_path.join("app");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("app.cppm"),
        r#"
            export module app;
            export import :core;
            import util;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("core.cppm"),
        r#"
            export module app:core;
            import util;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        test_path.join("main.cc"),
        r#"
            import app;
            import util;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    std::fs::write(path.join("deps"), "util").unwrap();
}

// A linear chain of three entries: `top` depends on `mid` and `mid` depends
// on `core`. `top` has a submodule that imports the root of `mid`. Source
// files of `mid` import `core` directly, but the files of `top` only import
// `mid` because only direct dependencies may be imported.
#[allow(dead_code)]
pub fn make_chain(src_path: &Path) {
    let path = src_path.join("core");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("core.cppm"),
        r#"
            export module core;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    std::fs::write(
        lib_path.join("base_utils.cc"),
        r#"
            import core;
        "#,
    )
    .unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        test_path.join("core.cc"),
        r#"
            import core;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    std::fs::write(path.join("deps"), "").unwrap();

    let path = src_path.join("mid");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("mid.cppm"),
        r#"
            export module mid;
            import core;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
        bin_path.join("run_42.cc"),
        r#"
            import mid;
            import core;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        test_path.join("mid.cc"),
        r#"
            import mid;
            import core;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    std::fs::write(path.join("deps"), "core").unwrap();

    let path = src_path.join("top");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("top.cppm"),
        r#"
            export module top;
            export import :extra;
            import mid;
        "#,
    )
    .unwrap();

    std::fs::write(
        mod_path.join("extra.cppm"),
        r#"
            export module top:extra;
            import mid;
        "#,
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
        bin_path.join("top_main.cc"),
        r#"
            import top;
            import mid;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        test_path.join("top.cc"),
        r#"
            import top;
            import mid;

            int main(void) {
                return 0;
            }
        "#,
    )
    .unwrap();

    std::fs::write(path.join("deps"), "mid").unwrap();
}

pub fn make_myapp(src_path: &Path) {
    let path = src_path.join("myapp");
    std::fs::create_dir(&path).unwrap();

    let mod_path = path.join("mod");
    std::fs::create_dir(&mod_path).unwrap();

    std::fs::write(
        mod_path.join("myapp.cppm"),
        "
            export module myapp;
            import base;
            import io;
        ",
    )
    .unwrap();

    let lib_path = path.join("lib");
    std::fs::create_dir(&lib_path).unwrap();

    let bin_path = path.join("bin");
    std::fs::create_dir(&bin_path).unwrap();

    std::fs::write(
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
    .unwrap();

    let test_path = path.join("test");
    std::fs::create_dir(&test_path).unwrap();

    std::fs::write(
        path.join("deps"),
        r#"
            base
            io
        "#,
    )
    .unwrap();
}
