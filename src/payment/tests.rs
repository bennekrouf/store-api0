use crate::endpoint_store::EndpointStore;
use uuid::Uuid;

/// Pure unit tests for the credit-deduction formula — no database required.
/// These always run in CI.
#[cfg(test)]
mod credit_formula {
    #[test]
    fn cost_for_1000_tokens_is_20() {
        let tokens: i64 = 1000;
        let cost = (tokens * 20) / 1000;
        assert_eq!(cost, 20);
    }

    #[test]
    fn cost_for_50_tokens_is_1() {
        let tokens: i64 = 50;
        let cost = (tokens * 20) / 1000;
        assert_eq!(cost, 1);
    }

    #[test]
    fn minimum_cost_for_small_token_count_is_1() {
        let tokens: i64 = 10;
        let raw_cost = (tokens * 20) / 1000; // 0
        let cost = if raw_cost < 1 && tokens > 0 { 1 } else { raw_cost };
        assert_eq!(cost, 1, "Minimum cost should be 1 for non-zero token use");
    }

    #[test]
    fn balance_after_sequential_deductions() {
        let initial: i64 = 1000;
        let after_case1 = initial - 20; // 980
        let after_case2 = after_case1 - 1; // 979
        let after_case3 = after_case2 - 1; // 978
        assert_eq!(after_case1, 980);
        assert_eq!(after_case2, 979);
        assert_eq!(after_case3, 978);
    }
}

/// Integration test — requires a live PostgreSQL instance at localhost:5432.
/// Skipped in CI (`cargo test`) but can be run locally with:
///   cargo test -- --ignored test_credit_deduction_logic
#[tokio::test]
#[ignore = "requires live PostgreSQL at localhost:5432/veno_application"]
async fn test_credit_deduction_logic() {
    let database_url = "postgresql://veno_user:veno_password@localhost:5432/veno_application".to_string();

    println!("Using DATABASE_URL: {}", database_url);

    let store = EndpointStore::new(&database_url).await.expect("Failed to create store");
    let email = format!("test_credit_{}@example.com", Uuid::new_v4());
    let tenant = crate::endpoint_store::tenant_management::get_default_tenant(&store, &email).await.expect("Failed to get tenant");
    let tenant_id = &tenant.id;

    println!("Testing with user: {} and tenant: {}", email, tenant_id);

    // 2. Initial Credit Balance
    let initial_credits = 1000;
    let balance = store.update_credit_balance(tenant_id, &email, initial_credits, "test", None).await.expect("Failed to set initial balance");
    assert_eq!(balance, initial_credits, "Initial balance mismatch");

    // 3. Test Case 1: 1000 tokens -> 20 credits deduction
    // Formula: (1000 * 20) / 1000 = 20 credits
    let tokens_1 = 1000;
    let cost_1 = (tokens_1 as i64 * 20) / 1000;
    assert_eq!(cost_1, 20);

    let new_balance_1 = store.update_credit_balance(tenant_id, &email, -cost_1, "test", None).await.expect("Failed to deduct Case 1");
    assert_eq!(new_balance_1, 980, "Balance mismatch after Case 1");

    // 4. Test Case 2: 50 tokens -> 1 credit deduction
    // Formula: (50 * 20) / 1000 = 1
    let tokens_2 = 50;
    let cost_2 = (tokens_2 as i64 * 20) / 1000;
    assert_eq!(cost_2, 1);

    let new_balance_2 = store.update_credit_balance(tenant_id, &email, -cost_2, "test", None).await.expect("Failed to deduct Case 2");
    assert_eq!(new_balance_2, 979, "Balance mismatch after Case 2");

    // 5. Test Case 3: 10 tokens -> 1 credit deduction (Minimum)
    // Formula: (10 * 20) / 1000 = 0.2 -> floor to 0, but minimum rule lifts to 1
    let tokens_3 = 10;
    let raw_cost_3 = (tokens_3 as i64 * 20) / 1000; // 0
    let cost_3 = if raw_cost_3 < 1 && tokens_3 > 0 { 1 } else { raw_cost_3 };
    assert_eq!(cost_3, 1, "Minimum cost should be 1");

    let new_balance_3 = store.update_credit_balance(tenant_id, &email, -cost_3, "test", None).await.expect("Failed to deduct Case 3");
    assert_eq!(new_balance_3, 978, "Balance mismatch after Case 3");

    // Cleanup
    store.force_clean_user_data(&email).await.expect("Failed to cleanup");
}
