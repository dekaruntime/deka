#[allow(clippy::module_inception)]
mod tests {
    use crate::audit::sha256_hex;
    use crate::auth::{
        PeerCredentials, agent_users_in_group, authorize_peer, forbidden_peer_reply,
        group_line_contains_user, peer_credentials, peer_credentials_from_ucred,
    };
    use crate::commands::validate_slug;
    use crate::config::{AppState, CommandOutput, CommandRunner, SystemCommandRunner};
    use crate::protocol::{ServiceReply, http_response, parse_http_request, parse_json};
    use crate::requests::{
        CreateAgentRequest, DeleteUnitRequest, SystemctlRequest, WriteUnitRequest,
    };
    use crate::server::handle_request;
    use crate::systemctl::{
        handle_systemctl_request, validate_systemctl_action, validate_systemctl_unit,
    };
    use crate::units::{
        atomic_write_unit, delete_unit, dispatcher_unit_name, handle_write_unit_request,
        render_dispatcher_unit, validate_unit_name, write_unit,
    };
    use std::collections::{HashMap, VecDeque};
    use std::fs;
    use std::io;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::{Barrier, Mutex};
    use std::time::SystemTime;
    use std::time::{Duration, Instant};
    use tokio::net::UnixStream;

    struct FakeRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        outputs: Mutex<VecDeque<io::Result<CommandOutput>>>,
    }

    impl FakeRunner {
        fn new(outputs: Vec<io::Result<CommandOutput>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(outputs.into()),
            })
        }

        fn success() -> CommandOutput {
            CommandOutput {
                success: true,
                exit: Some(0),
                stdout: "ok\n".to_string(),
                stderr: String::new(),
            }
        }

        fn exit(exit: i32, stdout: &str, stderr: &str) -> CommandOutput {
            CommandOutput {
                success: exit == 0,
                exit: Some(exit),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                args.iter().map(|arg| arg.to_string()).collect(),
            ));
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(Self::success()))
        }
    }

    fn test_state(command_runner: Arc<dyn CommandRunner>) -> AppState {
        let dir = std::env::temp_dir().join(format!(
            "gild-agent-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let passwd_path = dir.join("passwd");
        fs::write(&passwd_path, "root:x:0:0:root:/root:/bin/bash\n").unwrap();
        AppState {
            started_at: Instant::now(),
            socket_path: dir.join("gild-agent-test.sock"),
            orchestrator_gid: 1,
            audit_log_path: dir.join("gild-agent.log"),
            passwd_path,
            systemd_unit_root: dir.join("systemd"),
            systemd_unit_owner: None,
            command_runner,
        }
    }

    fn test_peer() -> PeerCredentials {
        PeerCredentials {
            pid: 4242,
            uid: 1001,
            gid: 1001,
        }
    }

    #[test]
    fn parses_post_request_with_json_body() {
        let body = "{\"slug\":\"agent-khalid\",\"persona_ref\":\"abc\"}";
        let raw = format!(
            "POST /v1/agent/create HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {body}",
            body.len()
        );
        let request = parse_http_request(&raw).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/agent/create");
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        let body: CreateAgentRequest = parse_json(&request).unwrap();
        assert_eq!(body.slug, "agent-khalid");
        assert_eq!(body.persona_ref, "abc");
    }

    #[test]
    fn rejects_short_body() {
        let err = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 99\r\n\r\n{\"slug\":\"a\"}",
        )
        .unwrap_err();
        assert!(err.to_string().contains("shorter than content-length"));
    }

    #[tokio::test]
    async fn health_response_shape() {
        let state = AppState {
            started_at: Instant::now(),
            socket_path: PathBuf::from("/tmp/gild-agent-test.sock"),
            orchestrator_gid: 1,
            audit_log_path: PathBuf::from("/tmp/gild-agent-test.log"),
            passwd_path: PathBuf::from("/tmp/gild-agent-test-passwd"),
            systemd_unit_root: std::env::temp_dir().join("gild-agent-test-systemd"),
            systemd_unit_owner: None,
            command_runner: Arc::new(SystemCommandRunner),
        };
        let request =
            parse_http_request("GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let reply = handle_request(&state, &request, test_peer()).await;
        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["uptime"].as_u64().is_some());
    }

    #[test]
    fn response_shape_includes_content_length() {
        let response = http_response(501, "{\"error\":\"not yet implemented\"}");
        assert!(response.starts_with("HTTP/1.1 501 Not Implemented"));
        assert!(response.contains("content-type: application/json"));
        assert!(response.ends_with("{\"error\":\"not yet implemented\"}"));
    }

    #[test]
    fn extracts_peercred_from_raw_ucred() {
        let cred = libc::ucred {
            pid: 123,
            uid: 456,
            gid: 789,
        };
        assert_eq!(
            peer_credentials_from_ucred(cred).unwrap(),
            PeerCredentials {
                pid: 123,
                uid: 456,
                gid: 789,
            }
        );
    }

    #[test]
    fn rejects_negative_peer_pid() {
        let cred = libc::ucred {
            pid: -1,
            uid: 456,
            gid: 789,
        };
        assert!(peer_credentials_from_ucred(cred).is_err());
    }

    #[test]
    fn authorize_peer_accepts_orchestrator_member() {
        let peer = PeerCredentials {
            pid: 123,
            uid: 1000,
            gid: 777,
        };

        assert!(authorize_peer(peer, 777).unwrap());
    }

    #[test]
    fn authorize_peer_rejects_gild_agents_member() {
        let peer = PeerCredentials {
            pid: 123,
            uid: 999_999,
            gid: 778,
        };

        assert!(!authorize_peer(peer, 777).unwrap());
    }

    #[test]
    fn forbidden_peer_reply_names_gild_agents_rejection() {
        let reply = forbidden_peer_reply();

        assert_eq!(reply.status, 403);
        assert!(reply.body.contains("gild-orchestrator"));
        assert!(reply.body.contains("gild-agents"));
        assert!(reply.body.contains("not authorized"));
    }

    #[tokio::test]
    async fn extracts_peercred_from_unix_socket() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let cred = peer_credentials(&stream).unwrap();
        assert_eq!(cred.pid, std::process::id());
        assert_eq!(cred.uid, unsafe { libc::geteuid() });
        assert_eq!(cred.gid, unsafe { libc::getegid() });
    }

    #[test]
    fn group_line_membership_parser() {
        assert_eq!(
            group_line_contains_user("gild-orchestrator:x:777:ava,agent-khalid", "agent-khalid"),
            Some(777)
        );
        assert_eq!(
            group_line_contains_user("gild-orchestrator:x:777:ava", "agent-khalid"),
            None
        );
    }

    #[test]
    fn startup_check_finds_agent_users_in_orchestrator() {
        let dir = std::env::temp_dir().join(format!(
            "gild-agent-groups-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let passwd = dir.join("passwd");
        let group = dir.join("group");
        fs::write(
            &passwd,
            "root:x:0:0:root:/root:/bin/bash\nagent-primary:x:1001:777::/home/agent-primary:/bin/bash\nagent-member:x:1002:1002::/home/agent-member:/bin/bash\nagent-safe:x:1003:1003::/home/agent-safe:/bin/bash\n",
        )
        .unwrap();
        fs::write(
            &group,
            "gild-orchestrator:x:777:ava,agent-member\ngild-agents:x:778:agent-safe\n",
        )
        .unwrap();

        let users = agent_users_in_group(&passwd, &group, "gild-orchestrator", 777).unwrap();

        assert_eq!(users, vec!["agent-member", "agent-primary"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validates_slug() {
        assert!(validate_slug("agent-khalid").is_ok());
        assert!(validate_slug("agent-zed").is_ok());
        assert!(validate_slug("agent-a1").is_ok());
        assert!(validate_slug("agent-a-b").is_ok());
        assert!(validate_slug("agent-khalid-123456789012345678").is_ok());
        assert!(validate_slug("khalid").is_err());
        assert!(validate_slug("agent-a").is_err());
        assert!(validate_slug("AgentKhalid").is_err());
        assert!(validate_slug("agent_khalid").is_err());
        assert!(validate_slug("agent-1a").is_err());
        assert!(validate_slug("agent-a_b").is_err());
        assert!(validate_slug("agent-khalid-1234567890123456789012345").is_err());
    }

    #[tokio::test]
    async fn create_agent_invokes_useradd_and_audits_peercred() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success()), Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/create HTTP/1.1\r\nContent-Length: 49\r\n\r\n{\"slug\":\"agent-zed\",\"persona_ref\":\"personas/zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [
                (
                    "/usr/sbin/groupadd".to_string(),
                    vec![
                        "--system".to_string(),
                        "--force".to_string(),
                        "gild-agents".to_string(),
                    ],
                ),
                (
                    "/usr/sbin/useradd".to_string(),
                    vec![
                        "--create-home".to_string(),
                        "--shell".to_string(),
                        "/bin/bash".to_string(),
                        "--gid".to_string(),
                        "gild-agents".to_string(),
                        "agent-zed".to_string(),
                    ],
                )
            ]
        );
        assert!(
            !runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(_, args)| args)
                .any(|arg| arg == "gild-orchestrator")
        );
        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let line: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(line["op"], "useradd");
        assert_eq!(line["slug"], "agent-zed");
        assert_eq!(line["exit"], 0);
        assert_eq!(line["by_uid"], 1001);
        assert_eq!(line["by_pid"], 4242);
        assert!(line["ts"].as_u64().is_some());
    }

    #[tokio::test]
    async fn create_agent_refuses_existing_user_without_useradd() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        fs::write(
            &state.passwd_path,
            "root:x:0:0:root:/root:/bin/bash\nagent-zed:x:1002:1002::/home/agent-zed:/bin/bash\n",
        )
        .unwrap();
        let request = parse_http_request(
            "POST /v1/agent/create HTTP/1.1\r\nContent-Length: 49\r\n\r\n{\"slug\":\"agent-zed\",\"persona_ref\":\"personas/zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 409);
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_agent_refuses_running_processes() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "123\n456\n", ""))]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 20\r\n\r\n{\"slug\":\"agent-zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 409);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/usr/bin/pgrep");
        assert_eq!(calls[0].1, vec!["-u".to_string(), "agent-zed".to_string()]);
        assert!(reply.body.contains("running processes"));
    }

    #[tokio::test]
    async fn remove_agent_invokes_userdel_after_empty_pgrep() {
        let runner = FakeRunner::new(vec![
            Ok(FakeRunner::exit(1, "", "")),
            Ok(FakeRunner::success()),
        ]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 20\r\n\r\n{\"slug\":\"agent-zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[1].0, "/usr/sbin/userdel");
        assert_eq!(
            calls[1].1,
            vec!["--remove".to_string(), "agent-zed".to_string()]
        );
    }

    #[test]
    fn validates_systemctl_action_whitelist() {
        for action in [
            "daemon-reload",
            "start",
            "stop",
            "restart",
            "reload",
            "enable",
            "disable",
            "status",
        ] {
            assert!(validate_systemctl_action(action).is_ok(), "{action}");
        }

        assert!(validate_systemctl_action("reboot").is_err());
        assert!(validate_systemctl_action("").is_err());
    }

    #[test]
    fn validates_systemctl_unit_pattern() {
        assert!(validate_systemctl_unit("gg.tana.agent-foo.service").is_ok());
        assert!(validate_systemctl_unit("gg.tana.agent.foo-1.timer").is_ok());
        assert!(validate_systemctl_unit("gg.tana.gild-dispatcher@agent-foo.service").is_ok());

        assert!(validate_systemctl_unit("nginx.service").is_err());
        assert!(validate_systemctl_unit("../../etc/passwd").is_err());
        assert!(validate_systemctl_unit("gg.tana.AgentFoo.service").is_err());
        assert!(validate_systemctl_unit("gg.tana.agent-foo.socket").is_err());
    }

    #[test]
    fn systemctl_handler_invokes_runner_and_audits() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "started\n", ""))]);
        let state = test_state(runner.clone());
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "start".to_string(),
            unit: "gg.tana.agent-foo.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["exit"], 0);
        assert_eq!(json["stdout"], "started\n");
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [(
                "/usr/bin/systemctl".to_string(),
                vec!["start".to_string(), "gg.tana.agent-foo.service".to_string()]
            )]
        );

        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let audit_json: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(audit_json["op"], "systemctl");
        assert_eq!(audit_json["action"], "start");
        assert_eq!(audit_json["unit"], "gg.tana.agent-foo.service");
        assert_eq!(audit_json["exit"], 0);
        assert_eq!(audit_json["by_uid"], 1001);
        assert_eq!(audit_json["by_pid"], 4242);
    }

    #[test]
    fn audit_log_is_created_with_0640_mode() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "started\n", ""))]);
        let state = test_state(runner);
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "start".to_string(),
            unit: "gg.tana.agent-foo.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 200);
        let mode = fs::metadata(&state.audit_log_path).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn systemctl_daemon_reload_omits_unit_arg_but_audits_unit_context() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "", ""))]);
        let state = test_state(runner.clone());
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "daemon-reload".to_string(),
            unit: "gg.tana.gild-dispatcher@agent-foo.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 200);
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [(
                "/usr/bin/systemctl".to_string(),
                vec!["daemon-reload".to_string()]
            )]
        );

        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let audit_json: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(audit_json["action"], "daemon-reload");
        assert_eq!(
            audit_json["unit"],
            "gg.tana.gild-dispatcher@agent-foo.service"
        );
    }

    #[test]
    fn systemctl_handler_rejects_invalid_request_before_runner() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "restart".to_string(),
            unit: "nginx.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 400);
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn validates_write_unit_name_pattern() {
        assert!(validate_unit_name("gg.tana.gild-dispatcher@agent-foo.service").is_ok());
        assert!(validate_unit_name("gg.tana.agent.foo-1.timer").is_ok());

        assert!(validate_unit_name("nginx.service").is_err());
        assert!(validate_unit_name("../../etc/passwd").is_err());
        assert!(validate_unit_name("gg.tana.AgentFoo.service").is_err());
        assert!(validate_unit_name("gg.tana.1agent.service").is_err());
        assert!(validate_unit_name("gg.tana.agent_foo.service").is_err());
        assert!(validate_unit_name("gg.tana.agent-foo.socket").is_err());
    }

    #[test]
    fn validates_delete_unit_name_pattern() {
        assert!(validate_unit_name("gg.tana.gild-dispatcher@agent-foo.service").is_ok());
        assert!(validate_unit_name("gg.tana.agent.foo-1.timer").is_ok());

        assert!(validate_unit_name("nginx.service").is_err());
        assert!(validate_unit_name("../../etc/passwd").is_err());
        assert!(validate_unit_name("gg.tana.AgentFoo.service").is_err());
        assert!(validate_unit_name("gg.tana.1agent.service").is_err());
        assert!(validate_unit_name("gg.tana.agent_foo.service").is_err());
        assert!(validate_unit_name("gg.tana.agent-foo.socket").is_err());
    }

    #[test]
    fn renders_dispatcher_unit_from_structured_request() {
        let request = WriteUnitRequest {
            op: "write_unit".to_string(),
            slug: "agent-foo".to_string(),
            kind: "dispatcher".to_string(),
            port: 9430,
            extra_env: HashMap::from([("GILD_MODE".to_string(), "sandbox".to_string())]),
        };
        let unit = render_dispatcher_unit(&request);

        assert!(unit.contains("User=agent-foo\n"));
        assert!(unit.contains("Group=agent-foo\n"));
        assert!(unit.contains(
            "ExecStart=/home/sami/.bun/bin/bun run /home/sami/Projects/tana/agent-dispatcher/src/index.ts\n"
        ));
        assert!(unit.contains("Environment=\"AGENT_DISPATCHER_PORT=9430\"\n"));
        assert!(unit.contains("Environment=\"AGENT_WORKER_SANDBOX=host\"\n"));
        assert!(unit.contains("Environment=\"GILD_SOCKET_PATH=/run/gild/sock\"\n"));
        assert!(unit.contains("Environment=GILD_MODE=sandbox\n"));
        assert!(!unit.contains("gild-orchestrator"));
    }

    #[test]
    fn write_unit_rejects_raw_contents_shape() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let body = serde_json::json!({
            "op": "write_unit",
            "unit": "gg.tana.gild-dispatcher@agent-bar.service",
            "contents": "[Unit]\nDescription=agent bar\n[Service]\nExecStart=/bin/true\n"
        })
        .to_string();
        let request = parse_http_request(&format!(
            "POST /v1/write_unit HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .unwrap();

        let parsed = parse_json::<WriteUnitRequest>(&request);

        assert!(parsed.is_err());
        assert!(!state.systemd_unit_root.exists());
    }

    #[test]
    fn write_unit_is_idempotent_but_rejects_different_existing_contents() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let audit = state.audit_log_path.clone();
        let request = WriteUnitRequest {
            op: "write_unit".to_string(),
            slug: "agent-foo".to_string(),
            kind: "dispatcher".to_string(),
            port: 9430,
            extra_env: HashMap::new(),
        };

        let first = write_unit(
            &request,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap();
        let second = write_unit(
            &request,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap();

        assert_eq!(first.sha256, second.sha256);
        let unit = dispatcher_unit_name(&request.slug);
        let path = state.systemd_unit_root.join(&unit);
        assert_eq!(first.path, path.display().to_string());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(!path.with_extension("service.tmp").exists());

        let changed = WriteUnitRequest {
            extra_env: HashMap::from([("CHANGED".to_string(), "yes".to_string())]),
            ..request
        };
        let err = write_unit(
            &changed,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("different sha256"));

        let audit_lines = fs::read_to_string(&audit).unwrap();
        assert_eq!(audit_lines.lines().count(), 2);
        let audit_json: serde_json::Value =
            serde_json::from_str(audit_lines.lines().next().unwrap()).unwrap();
        assert_eq!(audit_json["op"], "write_unit");
        assert_eq!(
            audit_json["unit"],
            "gg.tana.gild-dispatcher@agent-foo.service"
        );
        assert_eq!(audit_json["sha256"], first.sha256);
        assert_eq!(audit_json["by_uid"], 1001);
        assert_eq!(audit_json["by_pid"], 4242);
    }

    #[test]
    fn atomic_write_unit_same_sha_preserves_existing_file_metadata() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let unit = "gg.tana.gild-dispatcher@agent-idempotent.service";
        let path = state.systemd_unit_root.join(unit);
        let contents = b"[Unit]\nDescription=agent idempotent\n[Service]\nExecStart=/bin/true\n";

        atomic_write_unit(&path, contents, None).unwrap();
        let before = fs::metadata(&path).unwrap();

        std::thread::sleep(Duration::from_millis(5));
        atomic_write_unit(&path, contents, None).unwrap();

        let after = fs::metadata(&path).unwrap();
        assert_eq!(before.dev(), after.dev());
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.mtime(), after.mtime());
        assert_eq!(before.mtime_nsec(), after.mtime_nsec());

        let prefix = format!("{unit}.tmp.");
        let leftovers: Vec<PathBuf> = fs::read_dir(&state.systemd_unit_root)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name()?.to_str()?;
                name.starts_with(&prefix).then_some(path)
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover tmp files: {leftovers:?}");
    }

    #[test]
    fn concurrent_write_unit_same_unit_conflicts_without_tmp_orphans() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let unit = "gg.tana.gild-dispatcher@agent-race.service".to_string();
        let first_env = HashMap::from([("RACE".to_string(), "one".to_string())]);
        let second_env = HashMap::from([("RACE".to_string(), "two".to_string())]);
        let barrier = Arc::new(Barrier::new(3));

        let handles = [first_env.clone(), second_env.clone()].map(|extra_env| {
            let state = state.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let request = WriteUnitRequest {
                    op: "write_unit".to_string(),
                    slug: "agent-race".to_string(),
                    kind: "dispatcher".to_string(),
                    port: 9430,
                    extra_env,
                };
                barrier.wait();
                handle_write_unit_request(&state, &request, test_peer())
            })
        });

        barrier.wait();
        let replies: Vec<ServiceReply> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let mut statuses: Vec<u16> = replies.iter().map(|reply| reply.status).collect();
        statuses.sort_unstable();
        assert_eq!(statuses, vec![200, 409]);

        let target = state.systemd_unit_root.join(&unit);
        let written = fs::read_to_string(&target).unwrap();
        assert!(
            written.contains("Environment=RACE=one") || written.contains("Environment=RACE=two")
        );

        let prefix = format!("{unit}.tmp.");
        let leftovers: Vec<PathBuf> = fs::read_dir(&state.systemd_unit_root)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name()?.to_str()?;
                name.starts_with(&prefix).then_some(path)
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover tmp files: {leftovers:?}");
    }

    #[tokio::test]
    async fn write_unit_route_writes_to_configured_root() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let body = serde_json::json!({
            "op": "write_unit",
            "slug": "agent-bar",
            "kind": "dispatcher",
            "port": 9431,
            "extra_env": {"GILD_ENV": "test"}
        })
        .to_string();
        let raw = format!(
            "POST /v1/write_unit HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let request = parse_http_request(&raw).unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(
            json["path"],
            state
                .systemd_unit_root
                .join("gg.tana.gild-dispatcher@agent-bar.service")
                .display()
                .to_string()
        );
        assert!(json["sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn delete_unit_removes_existing_file_and_audits_removed_sha() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        fs::create_dir_all(&state.systemd_unit_root).unwrap();
        let unit = "gg.tana.gild-dispatcher@agent-delete.service";
        let path = state.systemd_unit_root.join(unit);
        let contents = b"[Unit]\nDescription=delete me\n[Service]\nExecStart=/bin/true\n";
        fs::write(&path, contents).unwrap();
        let request = DeleteUnitRequest {
            op: "delete_unit".to_string(),
            unit: unit.to_string(),
        };

        let reply = delete_unit(
            &request,
            &state.systemd_unit_root,
            &state.audit_log_path,
            test_peer(),
        )
        .unwrap();

        assert!(reply.existed);
        assert_eq!(reply.path, path.display().to_string());
        assert!(!path.exists());

        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let audit_json: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(audit_json["op"], "delete_unit");
        assert_eq!(audit_json["unit"], unit);
        assert_eq!(audit_json["sha256_removed"], sha256_hex(contents));
        assert_eq!(audit_json["existed"], true);
        assert_eq!(audit_json["by_uid"], 1001);
        assert_eq!(audit_json["by_pid"], 4242);
    }

    #[test]
    fn delete_unit_missing_is_idempotent_without_audit() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let unit = "gg.tana.gild-dispatcher@agent-missing.service";
        let request = DeleteUnitRequest {
            op: "delete_unit".to_string(),
            unit: unit.to_string(),
        };

        let reply = delete_unit(
            &request,
            &state.systemd_unit_root,
            &state.audit_log_path,
            test_peer(),
        )
        .unwrap();

        assert!(!reply.existed);
        assert_eq!(
            reply.path,
            state.systemd_unit_root.join(unit).display().to_string()
        );
        assert!(!state.audit_log_path.exists());
    }

    #[test]
    fn delete_unit_refuses_symlink_escape() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        fs::create_dir_all(&state.systemd_unit_root).unwrap();
        let outside = state
            .systemd_unit_root
            .parent()
            .unwrap()
            .join("outside.service");
        fs::write(&outside, "outside").unwrap();
        let unit = "gg.tana.gild-dispatcher@agent-link.service";
        let path = state.systemd_unit_root.join(unit);
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        let request = DeleteUnitRequest {
            op: "delete_unit".to_string(),
            unit: unit.to_string(),
        };

        let err = delete_unit(
            &request,
            &state.systemd_unit_root,
            &state.audit_log_path,
            test_peer(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("outside systemd unit root"));
        assert!(path.exists());
        assert!(outside.exists());
        assert!(!state.audit_log_path.exists());
    }

    #[tokio::test]
    async fn delete_unit_route_uses_configured_root() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        fs::create_dir_all(&state.systemd_unit_root).unwrap();
        let unit = "gg.tana.gild-dispatcher@agent-route.service";
        fs::write(state.systemd_unit_root.join(unit), "route").unwrap();
        let body = serde_json::json!({
            "op": "delete_unit",
            "unit": unit
        })
        .to_string();
        let raw = format!(
            "POST /v1/delete_unit HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let request = parse_http_request(&raw).unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["existed"], true);
        assert_eq!(
            json["path"],
            state.systemd_unit_root.join(unit).display().to_string()
        );
    }
}
