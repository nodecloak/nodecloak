use super::*;

fn upstream(port: u16, authenticated: bool) -> Upstream {
    Upstream {
        config: ProxyConfig {
            mode: "socks5".into(),
            host: "127.0.0.1".into(),
            port,
            username: if authenticated { "fixture-user" } else { "" }.into(),
            ..Default::default()
        },
        password: if authenticated {
            "fixture-password"
        } else {
            ""
        }
        .into(),
    }
}
async fn rejected_greeting(reply: &[u8], authenticated: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = reply.to_vec();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, if authenticated { 2 } else { 0 }]);
        socket.write_all(&reply).await.unwrap();
    });
    let error = tunnel(&upstream(port, authenticated), "target.invalid", 443)
        .await
        .err()
        .expect("server fixture must be rejected");
    server.await.unwrap();
    assert!(!error.contains("fixture-user"));
    assert!(!error.contains("fixture-password"));
    error
}
#[tokio::test]
async fn socks_closed_greeting_reports_the_response_stage_and_next_checks() {
    let error = rejected_greeting(&[], true).await;
    assert!(error.contains("握手响应"), "{error}");
    assert!(error.contains("连接已关闭"), "{error}");
    assert!(error.contains("协议和端口"), "{error}");
}
#[tokio::test]
async fn socks_non_socks_response_is_not_misreported_as_bad_authentication() {
    let error =
        rejected_greeting(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n", true).await;
    assert!(error.contains("不是 SOCKS5"), "{error}");
    assert!(error.contains("协议和端口"), "{error}");
}
#[tokio::test]
async fn socks_missing_credentials_and_rejected_methods_are_distinct() {
    let error = rejected_greeting(&[5, 2], false).await;
    assert!(error.contains("要求用户名和密码"), "{error}");
    let error = rejected_greeting(&[5, 255], true).await;
    assert!(error.contains("拒绝了认证方式"), "{error}");
}
#[tokio::test]
async fn socks_server_cannot_downgrade_an_authenticated_request_to_no_auth() {
    let error = rejected_greeting(&[5, 0], true).await;
    assert!(error.contains("认证方式"), "{error}");
}
#[tokio::test]
async fn socks_authentication_disconnect_is_distinct_from_password_rejection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        socket.write_all(&[5, 2]).await.unwrap();
        assert_eq!(socket.read_u8().await.unwrap(), 1);
        for expected in [b"fixture-user".as_slice(), b"fixture-password".as_slice()] {
            let count = socket.read_u8().await.unwrap() as usize;
            let mut value = vec![0; count];
            socket.read_exact(&mut value).await.unwrap();
            assert_eq!(value, expected);
        }
    });
    let error = tunnel(&upstream(port, true), "target.invalid", 443)
        .await
        .err()
        .unwrap();
    server.await.unwrap();
    assert!(error.contains("认证响应"), "{error}");
    assert!(error.contains("连接已关闭"), "{error}");
    assert!(!error.contains("fixture-password"));
}
