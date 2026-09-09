// PM2 process definition.
//
// Secrets are read from the environment, never written here: this file is in
// git, so anything literal in it is in the history too. Set them in the shell
// that starts PM2, or in an env file PM2 loads.
//
// Required: DATABASE_URL, API0_INTERNAL_SECRET (must match the gateway's),
// API0_ENCRYPTION_KEY (32 bytes base64 — without it the store refuses to store
// per-user credentials).
module.exports = {
    apps: [{
        name: "store",
        script: "./target/release/store",
        instances: 1,
        exec_mode: "fork",
        env: {
            NODE_ENV: "production",
            PORT: 50055,
            DATABASE_URL: process.env.DATABASE_URL,
            API0_INTERNAL_SECRET: process.env.API0_INTERNAL_SECRET,
            API0_ENCRYPTION_KEY: process.env.API0_ENCRYPTION_KEY,
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
