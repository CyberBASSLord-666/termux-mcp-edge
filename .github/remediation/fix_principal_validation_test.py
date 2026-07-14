#!/usr/bin/env python3
from pathlib import Path

path = Path("src/config.rs")
text = path.read_text()
old = '''        for invalid in ["", "contains whitespace", "slash/not-allowed", "é"] {
            config.auth.static_principal_id = Some(invalid.to_owned());
            let error = validate_runtime_auth_posture(&config).unwrap_err();
            assert!(!error.to_string().contains(invalid));
        }

        config.auth.static_principal_id = Some("configured-token".to_owned());
'''
new = '''        config.auth.static_principal_id = Some(String::new());
        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert!(error.to_string().contains("must not be empty"));

        for invalid in ["contains whitespace", "slash/not-allowed", "é"] {
            config.auth.static_principal_id = Some(invalid.to_owned());
            let error = validate_runtime_auth_posture(&config).unwrap_err();
            assert!(!error.to_string().contains(invalid));
        }

        config.auth.static_principal_id = Some("configured-token".to_owned());
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected one principal validation test block, found {count}")
path.write_text(text.replace(old, new, 1))
