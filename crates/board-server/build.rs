// 界面构建产物（web/dist）变化时重新编译，让网页端带上最新的界面
fn main() {
    println!("cargo:rerun-if-changed=../../web/dist");
}
