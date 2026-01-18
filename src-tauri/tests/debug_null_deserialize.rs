//! Debug test for null deserialization

use localrouter_ai::mcp::protocol::JsonRpcResponse;

#[test]
fn test_null_result_deserialization() {
    let json_with_null = r#"{"jsonrpc": "2.0", "id": 1, "result": null}"#;
    let response: JsonRpcResponse = serde_json::from_str(json_with_null).unwrap();

    println!("Deserialized response: {:?}", response);
    println!("result.is_some(): {}", response.result.is_some());
    println!("error.is_some(): {}", response.error.is_some());

    if let Some(ref result) = response.result {
        println!("Result value: {:?}", result);
        println!("Is null: {}", result.is_null());
    }

    // This should pass - null is a valid result
    assert!(response.result.is_some(), "Result should be Some(Value::Null)");
    assert!(response.error.is_none(), "Error should be None");
}

#[test]
fn test_missing_result_deserialization() {
    // What if result field is completely missing?
    let json_missing_result = r#"{"jsonrpc": "2.0", "id": 1}"#;
    let result: Result<JsonRpcResponse, _> = serde_json::from_str(json_missing_result);

    println!("Deserialization result: {:?}", result);

    // This might fail or give us None for both result and error
    if let Ok(response) = result {
        println!("result.is_some(): {}", response.result.is_some());
        println!("error.is_some(): {}", response.error.is_some());
    }
}
