# Board 服务端镜像：界面打包进 board-server 可执行文件，数据放在 /data。
# 部署见 docs/deploy.md。
#
# 两种构建方式（docker-compose.yml 用 BOARD_BUILD 选择）：
#   source    默认。在镜像里编译界面和 board-server，需要 2 GB 以上内存和磁盘
#   prebuilt  使用本机交叉编译好的 deploy/bin/board-server（小服务器用这种，见 deploy/build-linux.sh）

# ---- 界面 ----
FROM node:22-bookworm-slim AS web
WORKDIR /src
COPY web/package.json web/package-lock.json web/
RUN npm --prefix web ci
COPY web web
# 设置页内嵌了 SKILL.md
COPY skills skills
RUN npm --prefix web run build

# ---- 服务端 ----
FROM rust:1.91-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# 工作区成员；只编译 board-server，不编译桌面端
COPY src-tauri src-tauri
COPY --from=web /src/web/dist web/dist
RUN cargo build --release --locked -p board-server

# ---- 运行环境 ----
FROM debian:bookworm-slim AS runtime
# ca-certificates：访问 GitHub；tzdata：动态里的时间按 TZ 显示
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin board \
 && mkdir -p /data && chown board /data
ENV BOARD_DB=/data/board.db \
    BOARD_HOST=0.0.0.0 \
    BOARD_PORT=7420 \
    TZ=Asia/Shanghai \
    RUST_LOG=info
EXPOSE 7420
VOLUME /data
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["board-server", "healthcheck"]
ENTRYPOINT ["board-server"]

# ---- 用本机编译好的可执行文件 ----
FROM runtime AS prebuilt
COPY deploy/bin/board-server /usr/local/bin/board-server
USER board

# ---- 默认：在镜像里编译 ----
FROM runtime AS source
COPY --from=server /src/target/release/board-server /usr/local/bin/board-server
USER board
