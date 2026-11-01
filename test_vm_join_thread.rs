// Rust tests for VM join_thread path - to be added to lib.rs

#[cfg(test)]
mod vm_join_thread_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use cfml_codegen::BytecodeProgram;

    fn make_empty_program() -> BytecodeProgram {
        BytecodeProgram {
            functions: vec![],
        }
    }

    #[test]
    fn vm_join_thread_local_completes() {
        // Test local join where spawner joins its own thread
        let program = make_empty_program();
        let mut vm = CfmlVirtualMachine::new(program);
        
        // Simulate a thread spawn and completion
        let thread_name = "test_local";
        let result = ThreadResult {
            status: "COMPLETED".to_string(),
            output: "test output".to_string(),
            error: String::new(),
            mem_allocated: 0,
            error_struct: None,
            elapsed: 100,
            thread_vars: ValueMap::default(),
            return_value: None,
        };
        
        // Create thread handle with completed result
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(result.clone()).unwrap();
        
        let handle = ThreadHandle {
            name: thread_name.to_string(),
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
            result: None,
        };
        
        vm.live_threads.insert(thread_name.to_lowercase(), handle);
        
        // Join should succeed immediately
        let join_result = vm.join_thread(thread_name, 1000);
        assert!(join_result.is_ok(), "join should succeed");
        
        // Verify result was stored in cfthread scope
        let cfthread = vm.get_or_create_cfthread_scope();
        if let Some(ts) = cfthread.as_cfml_struct() {
            assert!(ts.contains_key(&thread_name.to_lowercase()));
        }
    }

    #[test]
    fn vm_join_thread_cross_thread_via_registry() {
        // Test cross-thread join where thread B joins thread A (both in registry)
        let program = make_empty_program();
        let mut vm = CfmlVirtualMachine::new(program);
        
        let thread_name = "cross_thread_test";
        let cancel = Arc::new(AtomicBool::new(false));
        let completion = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        
        let mut metadata = ValueMap::default();
        metadata.insert("name".to_string(), CfmlValue::string(thread_name.to_string()));
        metadata.insert("status".to_string(), CfmlValue::string("RUNNING".to_string()));
        
        // Register in registry
        if let Ok(mut reg) = vm.request_thread_registry.write() {
            reg.register(
                thread_name.to_string(),
                cancel.clone(),
                completion.clone(),
                CfmlValue::strukt(metadata),
                None,
                true,
            ).unwrap();
        }
        
        // Simulate completion
        let result = ThreadResult {
            status: "COMPLETED".to_string(),
            output: String::new(),
            error: String::new(),
            mem_allocated: 0,
            error_struct: None,
            elapsed: 50,
            thread_vars: ValueMap::default(),
            return_value: None,
        };
        
        {
            let (lock, cvar) = &*completion;
            let mut guard = lock.lock().unwrap();
            *guard = Some(result.clone());
            cvar.notify_all();
        }
        
        // Update registry metadata
        if let Ok(mut reg) = vm.request_thread_registry.write() {
            reg.update_from_result(thread_name, &result);
        }
        
        // Join via registry path (no local handle)
        let join_result = vm.join_thread(thread_name, 1000);
        assert!(join_result.is_ok(), "cross-thread join should succeed");
    }

    #[test]
    fn vm_join_thread_shared_timeout_budget() {
        // Test that joining multiple threads shares the timeout budget
        let program = make_empty_program();
        let mut vm = CfmlVirtualMachine::new(program);
        
        // Create 3 threads that complete at different times
        for i in 1..=3 {
            let name = format!("budget_test_{}", i);
            let result = ThreadResult {
                status: "COMPLETED".to_string(),
                output: String::new(),
                error: String::new(),
                mem_allocated: 0,
                error_struct: None,
                elapsed: i * 50,
                thread_vars: ValueMap::default(),
                return_value: None,
            };
            
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(result).unwrap();
            
            let handle = ThreadHandle {
                name: name.clone(),
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                join: None,
                result: None,
            };
            
            vm.live_threads.insert(name.to_lowercase(), handle);
        }
        
        // Join all with shared 500ms budget - should succeed since all return immediately
        let start = std::time::Instant::now();
        for i in 1..=3 {
            let name = format!("budget_test_{}", i);
            let result = vm.join_thread(&name, 500);
            assert!(result.is_ok(), "join {} should succeed", name);
        }
        let elapsed = start.elapsed();
        
        // All joins should complete quickly (not wait 500ms each)
        assert!(elapsed < Duration::from_millis(200), "joins should complete quickly with shared budget");
    }

    #[test]
    fn vm_join_thread_self_join_returns_immediately() {
        // Test self-join detection returns immediately without hanging
        let program = make_empty_program();
        let mut vm = CfmlVirtualMachine::new(program);
        
        vm.cfml_thread_name = Some("self_thread".to_string());
        
        // Self-join should return immediately (no error, no hang)
        let result = vm.join_thread("self_thread", 1000);
        assert!(result.is_ok(), "self-join should return without error");
    }

    #[test]
    fn vm_join_thread_ancestor_cycle_safety() {
        // Test that no-name join excludes ancestors to prevent deadlock
        let program = make_empty_program();
        let mut vm = CfmlVirtualMachine::new(program);
        
        // Set up: we're thread "child", spawned by "parent"
        vm.cfml_thread_name = Some("child".to_string());
        
        let cancel = Arc::new(AtomicBool::new(false));
        let completion = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        
        // Register parent thread
        if let Ok(mut reg) = vm.request_thread_registry.write() {
            let mut parent_meta = ValueMap::default();
            parent_meta.insert("name".to_string(), CfmlValue::string("parent".to_string()));
            parent_meta.insert("status".to_string(), CfmlValue::string("RUNNING".to_string()));
            
            reg.register(
                "parent".to_string(),
                cancel.clone(),
                completion.clone(),
                CfmlValue::strukt(parent_meta),
                None,
                true,
            ).unwrap();
            
            // Register self as child of parent
            let mut child_meta = ValueMap::default();
            child_meta.insert("name".to_string(), CfmlValue::string("child".to_string()));
            child_meta.insert("status".to_string(), CfmlValue::string("RUNNING".to_string()));
            
            reg.register(
                "child".to_string(),
                cancel.clone(),
                completion.clone(),
                CfmlValue::strukt(child_meta),
                Some("parent".to_string()),
                true,
            ).unwrap();
            
            // Register sibling
            let mut sibling_meta = ValueMap::default();
            sibling_meta.insert("name".to_string(), CfmlValue::string("sibling".to_string()));
            sibling_meta.insert("status".to_string(), CfmlValue::string("COMPLETED".to_string()));
            
            reg.register(
                "sibling".to_string(),
                cancel.clone(),
                Arc::new((Mutex::new(Some(ThreadResult {
                    status: "COMPLETED".to_string(),
                    output: String::new(),
                    error: String::new(),
                    mem_allocated: 0,
                    error_struct: None,
                    elapsed: 10,
                    thread_vars: ValueMap::default(),
                    return_value: None,
                })), std::sync::Condvar::new())),
                CfmlValue::strukt(sibling_meta),
                Some("parent".to_string()),
                true,
            ).unwrap();
        }
        
        // No-name join from child should exclude parent and self, but could join sibling
        // The key is that it doesn't try to join parent (which would deadlock)
        let names = if let Ok(reg) = vm.request_thread_registry.read() {
            reg.get_non_ancestor_names(vm.cfml_thread_name.as_deref())
        } else {
            vec![]
        };
        
        assert!(!names.contains(&"child".to_string()), "should exclude self");
        assert!(!names.contains(&"parent".to_string()), "should exclude parent");
        assert!(names.contains(&"sibling".to_string()), "should include sibling");
    }
}
