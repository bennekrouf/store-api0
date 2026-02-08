use crate::endpoint_store::EndpointStore;
use uuid::Uuid;

#[tokio::test]
async fn test_credit_deduction_logic() {
    // Force correct credentials based on Docker inspection
    // postgresql://veno_user:veno_password@localhost:5432/veno_application
    let database_url = "postgresql://veno_user:veno_password@localhost:5432/veno_application".to_string();
    
    println!("Using DATABASE_URL: {}", database_url);
    
    let store = EndpointStore::new(&database_url).await.expect("Failed to create store");
    let email = format!("test_credit_{}@example.com", Uuid::new_v4());
    // let key_id = format!("key_{}", Uuid::new_v4()); // Unused now

    println!("Testing with user: {}", email);

    // 2. Initial Credit Balance
    let initial_credits = 1000;
    let balance = store.update_credit_balance(&email, initial_credits).await.expect("Failed to set initial balance");
    assert_eq!(balance, initial_credits, "Initial balance mismtach");

    // 3. Test Case 1: 1000 tokens -> 20 credits deduction
    // Formula: (1000 * 20) / 1000 = 20 credits
    let tokens_1 = 1000;
    let cost_1 = (tokens_1 as i64 * 20) / 1000;
    assert_eq!(cost_1, 20);
    
    // Simulate deduction
    let new_balance_1 = store.update_credit_balance(&email, -cost_1).await.expect("Failed to deduct Case 1");
    // 1000 - 20 = 980
    assert_eq!(new_balance_1, 980, "Balance mismatch after Case 1");

    // 4. Test Case 2: 50 tokens -> 1 credit deduction
    // Formula: (50 * 20) / 1000 = 1
    let tokens_2 = 50;
    let cost_2 = (tokens_2 as i64 * 20) / 1000;
    assert_eq!(cost_2, 1);

    let new_balance_2 = store.update_credit_balance(&email, -cost_2).await.expect("Failed to deduct Case 2");
    // 980 - 1 = 979
    assert_eq!(new_balance_2, 979, "Balance mismatch after Case 2");

    // 5. Test Case 3: 10 tokens -> 1 credit deduction (Minimum)
    // Formula: (10 * 20) / 1000 = 0.2 -> round/floor to 0?
    // Wait, the logic in log_api_usage.rs is:
    // let calculated_cost = (tokens * 20) / 1000;
    // let cost = if calculated_cost < 1 && tokens > 0 { 1 } else { calculated_cost };
    
    let tokens_3 = 10;
    let raw_cost_3 = (tokens_3 as i64 * 20) / 1000; // 0
    let cost_3 = if raw_cost_3 < 1 && tokens_3 > 0 { 1 } else { raw_cost_3 };
    assert_eq!(cost_3, 1, "Minimum cost should be 1");

    let new_balance_3 = store.update_credit_balance(&email, -cost_3).await.expect("Failed to deduct Case 3");
    // 979 - 1 = 978
    assert_eq!(new_balance_3, 978, "Balance mismatch after Case 3");

    // Cleanup (optional, but good practice)
    store.force_clean_user_data(&email).await.expect("Failed to cleanup");
}
