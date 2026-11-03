use super::model::*;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: u32,
    pub token: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRequest {
    pub session: String,
    pub controller_port: u16,
    pub controller_secret: String,
    pub rules: Vec<ResolvedRule>,
    pub proxies: Vec<ResolvedProxy>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase", deny_unknown_fields)]
pub enum Reply {
    Ready { interface: String, luid: u64 },
    Stopped,
    Failure { message: String },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Control {
    Stop,
}
const LIMIT: usize = 1_048_576;
pub async fn read_frame<R: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    stream: &mut R,
) -> Result<T, String> {
    let length = stream
        .read_u32_le()
        .await
        .map_err(|_| "应用代理控制通道已关闭")? as usize;
    if length == 0 || length > LIMIT {
        return Err("应用代理控制数据超过限制".into());
    }
    let mut bytes = vec![0; length];
    let result = match stream.read_exact(&mut bytes).await {
        Ok(_) => serde_json::from_slice(&bytes).map_err(|_| "应用代理控制数据格式无效".to_owned()),
        Err(_) => Err("应用代理控制数据不完整".into()),
    };
    use zeroize::Zeroize;
    bytes.zeroize();
    result
}
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    stream: &mut W,
    value: &T,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "无法生成应用代理控制数据")?;
    let result = if bytes.is_empty() || bytes.len() > LIMIT {
        Err("应用代理控制数据超过限制".to_owned())
    } else {
        async {
            stream.write_u32_le(bytes.len() as u32).await?;
            stream.write_all(&bytes).await?;
            stream.flush().await
        }
        .await
        .map_err(|_| "应用代理控制通道已关闭".to_owned())
    };
    use zeroize::Zeroize;
    bytes.zeroize();
    result
}
pub fn authenticated(hello: &Hello, token: &str) -> bool {
    hello.protocol == 1
        && hello.token.len() == 64
        && token.len() == 64
        && hello.token.bytes().all(|b| b.is_ascii_hexdigit())
        && token.bytes().all(|b| b.is_ascii_hexdigit())
        && hello
            .token
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod checks {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn bounded_frames_roundtrip_and_reject_oversized_or_truncated_payloads() {
        let (mut client, mut server) = tokio::io::duplex(256);
        write_frame(
            &mut client,
            &Hello {
                protocol: 1,
                token: "a".repeat(64),
            },
        )
        .await
        .unwrap();
        let hello: Hello = read_frame(&mut server).await.unwrap();
        assert!(authenticated(&hello, &"a".repeat(64)));
        for bytes in [vec![255, 255, 255, 255], vec![5, 0, 0, 0, b'{', b'}']] {
            let (mut client, mut server) = tokio::io::duplex(256);
            client.write_all(&bytes).await.unwrap();
            client.shutdown().await.unwrap();
            assert!(read_frame::<_, Hello>(&mut server).await.is_err());
        }
    }
    #[tokio::test]
    async fn oversized_outbound_frame_never_enters_pipe() {
        let (mut client, _server) = tokio::io::duplex(256);
        assert!(write_frame(&mut client, &"a".repeat(1_048_577))
            .await
            .is_err());
    }
    #[test]
    fn authentication_requires_protocol_and_exact_fresh_token() {
        let hello = Hello {
            protocol: 1,
            token: "a".repeat(64),
        };
        assert!(authenticated(&hello, &"a".repeat(64)));
        assert!(!authenticated(&hello, &"b".repeat(64)));
        assert!(!authenticated(
            &Hello {
                protocol: 2,
                token: "a".repeat(64)
            },
            &"a".repeat(64)
        ));
        assert!(!authenticated(
            &Hello {
                protocol: 1,
                token: String::new()
            },
            ""
        ));
    }
}
