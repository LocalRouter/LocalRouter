//! Debug test to investigate ID matching bug

use serde_json::{json, Value};

#[test]
fn test_value_to_string_behavior() {
    // Test how different Value types convert to string
    let num_value = Value::Number(1.into());
    let str_value = Value::String("1".to_string());

    println!("Number(1).to_string() = '{}'", num_value);
    println!("String('1').to_string() = '{}'", str_value);

    // Simulate what happens in the code
    let request_id_from_number = json!(1);
    let request_id_str = request_id_from_number.to_string();
    println!("Request ID string: '{}'", request_id_str);

    // Simulate response coming back
    let response_json = r#"{"id": 1}"#;
    let response: serde_json::Value = serde_json::from_str(response_json).unwrap();
    let response_id_str = response["id"].to_string();
    println!("Response ID string: '{}'", response_id_str);

    println!("Do they match? {}", request_id_str == response_id_str);

    assert_eq!(request_id_str, response_id_str, "IDs should match!");
}
