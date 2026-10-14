pub mod admin;
pub mod balance;
pub mod transactions;
// `payment::payment` reads oddly but is what every caller imports; renaming it
// is churn for a lint.
#[allow(clippy::module_inception)]
pub mod payment;
pub mod service;
pub mod topup;
pub mod license;
pub mod webhook;
pub mod update_balance;
#[cfg(test)]
pub mod tests;
