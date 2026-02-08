module.exports = {
    apps: [{
        name: "store",
        script: "./target/release/store",
        instances: 1,
        exec_mode: "fork",
        env: {
            NODE_ENV: "production",
            PORT: 50055,
            DATABASE_URL: "postgresql://api_store_prod_user:Salma2025!@localhost:5432/api-store-prod",
            CONFIG_PATH: "./config.yaml",
            LOG_PATH_API0: "/var/log/api0.log",
            ENDPOINTS_CONFIG_PATH: "endpoints.yaml",
            RUST_LOG: "debug",
            RUST_BACKTRACE: "1"
        },
        error_file: "./logs/store-error.log",
        out_file: "./logs/store-out.log",
        log_file: "./logs/store-combined.log",
        time: true,
        max_memory_restart: "500M"
    }]
};
