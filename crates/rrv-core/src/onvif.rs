//! ONVIF só para **descobrir e cadastrar** câmeras (plano D1): achar as da rede (WS-Discovery) e, com
//! usuário e senha, pedir os perfis de vídeo e as URLs RTSP (Profile S/T) para montar o `[[cameras]]`.
//! Nada aqui move a câmera nem muda a configuração dela: só leitura.
//!
//! A metade de cima é **pura** (mensagens SOAP, cabeçalho de segurança, leitura das respostas) e é testada
//! com respostas reais de uma Intelbras. A de baixo fala com a rede: sondas UDP (multicast **e**
//! unicast para cada endereço da sub-rede, porque muitos roteadores Wi-Fi não repassam multicast) e o SOAP
//! por HTTP via `curl` (como o webhook: sem biblioteca HTTP, e o corpo vai pela entrada padrão, nunca em
//! argumento de linha de comando). **A senha nunca é registrada nem impressa**: no SOAP vai só o digest.

use std::collections::HashSet;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};

const WSD_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const WSD_PORT: u16 = 3702;

/// Uma câmera achada na rede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// `http://ip/onvif/device_service`.
    pub xaddr: String,
    pub ip: String,
    pub name: Option<String>,
    pub hardware: Option<String>,
}

// ───────────────────────────── pura ─────────────────────────────

pub fn probe_message(message_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope" xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><e:Header><w:MessageID>uuid:{message_id}</w:MessageID><w:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To><w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header><e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>"#
    )
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn text_of(doc: &roxmltree::Document, tag: &str) -> Option<String> {
    doc.descendants()
        .find(|n| n.is_element() && local(n) == tag)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A resposta de uma sonda → a câmera, se for uma. `from_ip` é o remetente (usado se o `XAddrs` vier
/// sem um endereço utilizável).
pub fn parse_probe_match(xml: &str, from_ip: &str) -> Option<Found> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let addrs = text_of(&doc, "XAddrs")?;
    // vários endereços separados por espaço: o primeiro http IPv4 serve
    let xaddr = addrs
        .split_whitespace()
        .find(|a| a.starts_with("http://") || a.starts_with("https://"))?
        .to_string();
    let scopes = text_of(&doc, "Scopes").unwrap_or_default();
    let scope = |kind: &str| {
        scopes
            .split_whitespace()
            .find_map(|s| s.strip_prefix(&format!("onvif://www.onvif.org/{kind}/")))
            .map(percent_decode)
    };
    let ip = host_of(&xaddr).unwrap_or_else(|| from_ip.to_string());
    Some(Found {
        xaddr,
        ip,
        name: scope("name"),
        hardware: scope("hardware"),
    })
}

/// `http://host[:porta]/caminho` → `host`.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    (!host.is_empty()).then(|| host.to_string())
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn iso_utc(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// O cabeçalho WS-Security `UsernameToken` com senha em digest:
/// `base64(sha1(nonce + created + senha))`. O nonce vai em base64.
pub fn security_header(user: &str, password: &str, nonce: &[u8], created_unix: i64) -> String {
    let created = iso_utc(created_unix);
    let mut h = Sha1::new();
    h.update(nonce);
    h.update(created.as_bytes());
    h.update(password.as_bytes());
    let digest = base64(&h.finalize());
    format!(
        r#"<s:Header><Security s:mustUnderstand="1" xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd"><UsernameToken><Username>{}</Username><Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">{digest}</Password><Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">{}</Nonce><Created xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">{created}</Created></UsernameToken></Security></s:Header>"#,
        xml_escape(user),
        base64(nonce)
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Um envelope SOAP 1.2; `header` é o `<s:Header>` (vazio nas chamadas sem autenticação).
pub fn envelope(header: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:tds="http://www.onvif.org/ver10/device/wsdl" xmlns:trt="http://www.onvif.org/ver10/media/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema">{header}<s:Body>{body}</s:Body></s:Envelope>"#
    )
}

pub const BODY_DATE_TIME: &str = "<tds:GetSystemDateAndTime/>";
pub const BODY_DEVICE_INFO: &str = "<tds:GetDeviceInformation/>";
pub const BODY_CAPABILITIES: &str =
    "<tds:GetCapabilities><tds:Category>Media</tds:Category></tds:GetCapabilities>";
pub const BODY_PROFILES: &str = "<trt:GetProfiles/>";

pub fn body_stream_uri(profile_token: &str) -> String {
    format!(
        "<trt:GetStreamUri><trt:StreamSetup><tt:Stream>RTP-Unicast</tt:Stream><tt:Transport><tt:Protocol>RTSP</tt:Protocol></tt:Transport></trt:StreamSetup><trt:ProfileToken>{}</trt:ProfileToken></trt:GetStreamUri>",
        xml_escape(profile_token)
    )
}

/// A hora UTC da câmera (Unix s) de uma resposta de `GetSystemDateAndTime`.
pub fn parse_utc_time(xml: &str) -> Option<i64> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let utc = doc
        .descendants()
        .find(|n| n.is_element() && local(n) == "UTCDateTime")?;
    let num = |tag: &str| -> Option<u32> {
        utc.descendants()
            .find(|n| n.is_element() && local(n) == tag)?
            .text()?
            .trim()
            .parse()
            .ok()
    };
    let date = chrono::NaiveDate::from_ymd_opt(num("Year")? as i32, num("Month")?, num("Day")?)?;
    let time = date.and_hms_opt(num("Hour")?, num("Minute")?, num("Second")?)?;
    Some(time.and_utc().timestamp())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub token: String,
    pub name: String,
    pub encoding: String,
    pub width: u32,
    pub height: u32,
}

pub fn parse_profiles(xml: &str) -> Vec<Profile> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let child_text = |n: &roxmltree::Node, tag: &str| -> Option<String> {
        n.descendants()
            .find(|c| c.is_element() && local(c) == tag)
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    doc.descendants()
        .filter(|n| n.is_element() && local(n) == "Profiles")
        .filter_map(|p| {
            let token = p.attribute("token")?.to_string();
            let video = p
                .descendants()
                .find(|c| c.is_element() && local(c) == "VideoEncoderConfiguration")?;
            Some(Profile {
                token,
                name: child_text(&p, "Name").unwrap_or_default(),
                encoding: child_text(&video, "Encoding").unwrap_or_default(),
                width: child_text(&video, "Width")?.parse().ok()?,
                height: child_text(&video, "Height")?.parse().ok()?,
            })
        })
        .collect()
}

/// O endereço do serviço de mídia, de `GetCapabilities`.
pub fn parse_media_xaddr(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let media = doc
        .descendants()
        .find(|n| n.is_element() && local(n) == "Media")?;
    media
        .descendants()
        .find(|n| n.is_element() && local(n) == "XAddr")
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

pub fn parse_stream_uri(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    text_of(&doc, "Uri")
}

/// Fabricante e modelo, de `GetDeviceInformation`.
pub fn parse_device_info(xml: &str) -> Option<(String, String)> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    Some((text_of(&doc, "Manufacturer")?, text_of(&doc, "Model")?))
}

/// O motivo de uma resposta de erro SOAP (`Fault`), em palavras que a pessoa entende.
pub fn parse_fault(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    doc.descendants()
        .find(|n| n.is_element() && local(n) == "Fault")?;
    let reason = text_of(&doc, "Text").unwrap_or_default();
    let lower = reason.to_lowercase();
    Some(
        if lower.contains("not authorized")
            || lower.contains("sender not")
            || lower.contains("auth")
        {
            "usuário ou senha recusados pela câmera".to_string()
        } else if reason.is_empty() {
            "a câmera respondeu com um erro".to_string()
        } else {
            format!("a câmera respondeu com um erro: {reason}")
        },
    )
}

/// Principal (maior resolução) e secundário (a menor com ao menos 320 de largura, se for outra) dos
/// perfis de vídeo H.264/H.265. Perfis sem esses codecs (JPEG, áudio) ficam de fora.
pub fn pick_main_and_sub(profiles: &[Profile]) -> Option<(Profile, Option<Profile>)> {
    let mut video: Vec<&Profile> = profiles
        .iter()
        .filter(|p| {
            matches!(
                p.encoding.to_ascii_uppercase().as_str(),
                "H264" | "H265" | "HEVC"
            )
        })
        .collect();
    video.sort_by_key(|p| std::cmp::Reverse(u64::from(p.width) * u64::from(p.height)));
    let main = (*video.first()?).clone();
    let sub = video
        .iter()
        .rev()
        .find(|p| p.token != main.token && p.width >= 320)
        .map(|p| (*p).clone());
    Some((main, sub))
}

/// `rtsp://host/…` → `rtsp://usuario:${SEGREDO}@host/…` (o segredo é o nome de uma variável/arquivo de
/// segredo, nunca a senha). Se a URI já traz credenciais, são trocadas.
pub fn with_secret(uri: &str, user: &str, secret: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_string();
    };
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    format!("{scheme}://{user}:${{{secret}}}@{rest}")
}

/// `Intelbras iMX-C-309V` → `cam_intelbras_imx_c_309v_password`.
pub fn secret_name(display: &str) -> String {
    format!("cam_{}_password", crate::mqtt::slug(display))
}

// ───────────────────────────── rede ─────────────────────────────

/// `(interface, endereço, prefixo)` dos IPv4 locais, de `ip -4 -o addr`.
fn local_ipv4() -> Vec<(String, Ipv4Addr, u8)> {
    let Ok(out) = Command::new("ip")
        .args(["-4", "-o", "addr", "show"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let _idx = it.next()?;
            let name = it.next()?.to_string();
            let _inet = it.next()?;
            let (ip, prefix) = it.next()?.split_once('/')?;
            Some((
                name,
                ip.parse::<Ipv4Addr>().ok()?,
                prefix.parse::<u8>().ok()?,
            ))
        })
        .filter(|(name, ip, _)| {
            !ip.is_loopback()
                && !name.starts_with("docker")
                && !name.starts_with("br-")
                && !name.starts_with("veth")
        })
        .collect()
}

/// Os endereços da sub-rede para a varredura unicast (sem o próprio): só até /22 (≤ 1022).
fn subnet_hosts(ip: Ipv4Addr, prefix: u8) -> Vec<Ipv4Addr> {
    if !(22..=30).contains(&prefix) {
        return Vec::new();
    }
    let mask = u32::MAX << (32 - prefix);
    let base = u32::from(ip) & mask;
    let size = 1u32 << (32 - prefix);
    (1..size - 1)
        .map(|i| Ipv4Addr::from(base + i))
        .filter(|a| *a != ip)
        .collect()
}

/// Procura câmeras ONVIF por `timeout`: sonda multicast em cada interface e sonda unicast em cada
/// endereço da sub-rede (a segunda acha câmeras que o roteador Wi-Fi não deixa o multicast alcançar).
pub fn discover(timeout: Duration) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    let mut seen = HashSet::new();
    let mut sockets = Vec::new();
    for (_, ip, prefix) in local_ipv4() {
        let Ok(sock) = UdpSocket::bind(SocketAddrV4::new(ip, 0)) else {
            continue;
        };
        let _ = sock.set_read_timeout(Some(Duration::from_millis(100)));
        let _ = socket_multicast_if(&sock, ip);
        let probe = probe_message(&new_uuid());
        let _ = sock.send_to(probe.as_bytes(), SocketAddrV4::new(WSD_ADDR, WSD_PORT));
        for host in subnet_hosts(ip, prefix) {
            let _ = sock.send_to(probe.as_bytes(), SocketAddrV4::new(host, WSD_PORT));
        }
        sockets.push(sock);
    }
    let end = Instant::now() + timeout;
    let mut buf = vec![0u8; 65535];
    while Instant::now() < end {
        for sock in &sockets {
            if let Ok((n, from)) = sock.recv_from(&mut buf) {
                let text = String::from_utf8_lossy(&buf[..n]);
                if let Some(f) = parse_probe_match(&text, &from.ip().to_string())
                    && seen.insert(f.xaddr.clone())
                {
                    found.push(f);
                }
            }
        }
    }
    found.sort_by(|a, b| a.ip.cmp(&b.ip));
    found
}

fn socket_multicast_if(sock: &UdpSocket, ip: Ipv4Addr) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let addr = libc::in_addr {
        s_addr: u32::from(ip).to_be(),
    };
    // SAFETY: `addr` é uma `in_addr` válida e o tamanho passado é o dela.
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_MULTICAST_IF,
            (&raw const addr).cast(),
            std::mem::size_of::<libc::in_addr>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn new_uuid() -> String {
    let mut b = [0u8; 16];
    fill_random(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn fill_random(buf: &mut [u8]) {
    use std::io::Read;
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .is_err()
    {
        // sem /dev/urandom (não deveria acontecer no Linux): a hora e o pid bastam para um nonce
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((t >> ((i % 16) * 8)) as u8) ^ (std::process::id() as u8).wrapping_add(i as u8);
        }
    }
}

/// POST de um envelope SOAP; a resposta é o corpo, mesmo num `Fault` (que traz o motivo).
fn post(url: &str, envelope: &str) -> Result<String, String> {
    let mut child = Command::new("curl")
        .args([
            "-s",
            "-m",
            "8",
            "-H",
            "Content-Type: application/soap+xml; charset=utf-8",
            "--data-binary",
            "@-",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("não consegui rodar o curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(envelope.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    if body.trim().is_empty() {
        return Err(format!(
            "sem resposta de {}",
            crate::webhook::safe_target(url)
        ));
    }
    Ok(body)
}

/// Uma stream que a câmera oferece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub profile: Profile,
    /// `rtsp://host:porta/caminho`, **sem** credenciais.
    pub uri: String,
}

/// O que se descobriu de uma câmera com usuário e senha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub manufacturer: String,
    pub model: String,
    pub main: Stream,
    pub sub: Option<Stream>,
}

/// Pergunta à câmera quem ela é e quais streams oferece. Só leituras.
pub fn inspect(found: &Found, user: &str, password: &str) -> Result<Inspection, String> {
    // o digest depende da hora da câmera, não da nossa
    let offset = post(&found.xaddr, &envelope("", BODY_DATE_TIME))
        .ok()
        .and_then(|x| parse_utc_time(&x))
        .map_or(0, |cam| cam - chrono::Utc::now().timestamp());
    let call = |url: &str, body: &str| -> Result<String, String> {
        let mut nonce = [0u8; 16];
        fill_random(&mut nonce);
        let now = chrono::Utc::now().timestamp() + offset;
        let xml = post(
            url,
            &envelope(&security_header(user, password, &nonce, now), body),
        )?;
        match parse_fault(&xml) {
            Some(reason) => Err(reason),
            None => Ok(xml),
        }
    };
    let (manufacturer, model) = call(&found.xaddr, BODY_DEVICE_INFO)
        .ok()
        .and_then(|x| parse_device_info(&x))
        .unwrap_or_default();
    // a primeira chamada autenticada diz se o usuário/senha servem
    let caps = call(&found.xaddr, BODY_CAPABILITIES)?;
    let media = parse_media_xaddr(&caps).unwrap_or_else(|| found.xaddr.clone());
    let profiles = parse_profiles(&call(&media, BODY_PROFILES)?);
    let (main, sub) =
        pick_main_and_sub(&profiles).ok_or("a câmera não tem perfil de vídeo H.264/H.265")?;
    let stream = |p: Profile| -> Result<Stream, String> {
        let uri = parse_stream_uri(&call(&media, &body_stream_uri(&p.token))?)
            .ok_or("a câmera não devolveu a URL do stream")?;
        Ok(Stream { profile: p, uri })
    };
    Ok(Inspection {
        manufacturer,
        model,
        main: stream(main)?,
        sub: sub.map(stream).transpose()?,
    })
}

fn display_name(found: &Found, i: &Inspection) -> String {
    found
        .name
        .clone()
        .or_else(|| (!i.manufacturer.is_empty()).then(|| format!("{} {}", i.manufacturer, i.model)))
        .unwrap_or_else(|| found.ip.clone())
}

/// O nome do segredo que o trecho de `config_snippet` usa para a senha desta câmera.
pub fn secret_name_for(found: &Found, i: &Inspection) -> String {
    secret_name(&format!("{} {}", display_name(found, i), found.ip))
}

/// O trecho de `config.toml` de uma câmera inspecionada; a senha entra como `${SEGREDO}`.
pub fn config_snippet(found: &Found, user: &str, i: &Inspection) -> String {
    let display = display_name(found, i);
    let secret = secret_name_for(found, i);
    let mut out = format!(
        "[[cameras]]\nname = \"{}\"\nurl = \"{}\"\n",
        display.replace('"', "'"),
        with_secret(&i.main.uri, user, &secret)
    );
    if let Some(sub) = &i.sub {
        out.push_str(&format!(
            "sub_url = \"{}\"\n",
            with_secret(&sub.uri, user, &secret)
        ));
    }
    out.push_str(&format!(
        "# a senha vai no segredo `{secret}`: `rrvctl secret set {secret}` (chaveiro), variável de ambiente ou arquivo em /run/secrets\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resposta real de uma Intelbras iMX-C-309V a uma sonda WS-Discovery.
    const PROBE_MATCH: &str = r#"<?xml version="1.0" encoding="utf-8" standalone="yes" ?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><s:Header><a:MessageID>uuid:b8633bb8</a:MessageID></s:Header><s:Body><d:ProbeMatches><d:ProbeMatch><a:EndpointReference><a:Address>uuid:588bcacf</a:Address></a:EndpointReference><d:Types>dn:NetworkVideoTransmitter tds:Device</d:Types><d:Scopes>onvif://www.onvif.org/location/country/Brasil onvif://www.onvif.org/name/IntelBras onvif://www.onvif.org/hardware/iMX-C-309V onvif://www.onvif.org/Profile/Streaming</d:Scopes><d:XAddrs>http://192.168.1.46/onvif/device_service</d:XAddrs></d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>"#;

    /// Idem, `GetSystemDateAndTime` (sem autenticação).
    const DATE_TIME: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:tt="http://www.onvif.org/ver10/schema" xmlns:tds="http://www.onvif.org/ver10/device/wsdl"><s:Body><tds:GetSystemDateAndTimeResponse><tds:SystemDateAndTime><tt:DateTimeType>Manual</tt:DateTimeType><tt:TimeZone><tt:TZ>GMT-03:00</tt:TZ></tt:TimeZone><tt:UTCDateTime><tt:Time><tt:Hour>12</tt:Hour><tt:Minute>25</tt:Minute><tt:Second>41</tt:Second></tt:Time><tt:Date><tt:Year>2026</tt:Year><tt:Month>10</tt:Month><tt:Day>6</tt:Day></tt:Date></tt:UTCDateTime></tds:SystemDateAndTime></tds:GetSystemDateAndTimeResponse></s:Body></s:Envelope>"#;

    // Formato da especificação ONVIF (Profile S/T) para as chamadas que exigem senha.
    const PROFILES: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:trt="http://www.onvif.org/ver10/media/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><s:Body><trt:GetProfilesResponse>
<trt:Profiles token="Profile_1" fixed="true"><tt:Name>mainStream</tt:Name><tt:VideoEncoderConfiguration token="VEC_1"><tt:Name>VideoEncoder_1</tt:Name><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>1920</tt:Width><tt:Height>1080</tt:Height></tt:Resolution></tt:VideoEncoderConfiguration></trt:Profiles>
<trt:Profiles token="Profile_2" fixed="true"><tt:Name>subStream</tt:Name><tt:VideoEncoderConfiguration token="VEC_2"><tt:Name>VideoEncoder_2</tt:Name><tt:Encoding>H264</tt:Encoding><tt:Resolution><tt:Width>640</tt:Width><tt:Height>360</tt:Height></tt:Resolution></tt:VideoEncoderConfiguration></trt:Profiles>
<trt:Profiles token="Profile_3"><tt:Name>jpeg</tt:Name><tt:VideoEncoderConfiguration token="VEC_3"><tt:Encoding>JPEG</tt:Encoding><tt:Resolution><tt:Width>352</tt:Width><tt:Height>288</tt:Height></tt:Resolution></tt:VideoEncoderConfiguration></trt:Profiles>
<trt:Profiles token="Audio_only"><tt:Name>audio</tt:Name></trt:Profiles>
</trt:GetProfilesResponse></s:Body></s:Envelope>"#;

    const CAPABILITIES: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:tds="http://www.onvif.org/ver10/device/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><s:Body><tds:GetCapabilitiesResponse><tds:Capabilities><tt:Device><tt:XAddr>http://192.168.1.46/onvif/device_service</tt:XAddr></tt:Device><tt:Media><tt:XAddr>http://192.168.1.46/onvif/media_service</tt:XAddr></tt:Media></tds:Capabilities></tds:GetCapabilitiesResponse></s:Body></s:Envelope>"#;

    const STREAM_URI: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:trt="http://www.onvif.org/ver10/media/wsdl" xmlns:tt="http://www.onvif.org/ver10/schema"><s:Body><trt:GetStreamUriResponse><trt:MediaUri><tt:Uri>rtsp://192.168.1.46:554/cam/realmonitor?channel=1&amp;subtype=0</tt:Uri><tt:InvalidAfterConnect>false</tt:InvalidAfterConnect></trt:MediaUri></trt:GetStreamUriResponse></s:Body></s:Envelope>"#;

    const FAULT: &str = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><s:Fault><s:Code><s:Value>s:Sender</s:Value><s:Subcode><s:Value>ter:NotAuthorized</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang="en">Sender not Authorized</s:Text></s:Reason></s:Fault></s:Body></s:Envelope>"#;

    #[test]
    fn a_real_probe_match_gives_the_address_name_and_model() {
        let f = parse_probe_match(PROBE_MATCH, "9.9.9.9").unwrap();
        assert_eq!(f.xaddr, "http://192.168.1.46/onvif/device_service");
        assert_eq!(f.ip, "192.168.1.46");
        assert_eq!(f.name.as_deref(), Some("IntelBras"));
        assert_eq!(f.hardware.as_deref(), Some("iMX-C-309V"));
        assert!(parse_probe_match("<x/>", "1.1.1.1").is_none());
        assert!(parse_probe_match("lixo", "1.1.1.1").is_none());
    }

    #[test]
    fn scope_values_are_percent_decoded() {
        let xml = PROBE_MATCH.replace("name/IntelBras", "name/Port%C3%A3o%20Frente");
        assert_eq!(
            parse_probe_match(&xml, "1.1.1.1").unwrap().name.as_deref(),
            Some("Portão Frente")
        );
    }

    #[test]
    fn the_probe_message_asks_for_network_video_transmitters() {
        let m = probe_message("abc");
        assert!(m.contains("uuid:abc") && m.contains("NetworkVideoTransmitter"));
        assert!(roxmltree::Document::parse(&m).is_ok(), "XML bem formado");
    }

    #[test]
    fn the_camera_clock_is_read_from_the_utc_block() {
        assert_eq!(parse_utc_time(DATE_TIME), Some(1_791_289_541));
        assert_eq!(parse_utc_time("<x/>"), None);
    }

    #[test]
    fn the_password_digest_matches_an_independent_sha1() {
        // calculado à parte com hashlib: base64(sha1(nonce + created + senha))
        let nonce = [
            44, 170, 136, 232, 111, 192, 138, 66, 130, 66, 179, 116, 206, 166, 69, 150,
        ];
        let h = security_header("admin", "taadtaadpstcsm", &nonce, 1_284_623_445);
        assert!(h.contains("4yw0wCaNY3YRjgmVN3nmKpEImu8="), "{h}");
        assert!(h.contains("LKqI6G/AikKCQrN0zqZFlg=="), "nonce em base64");
        assert!(h.contains("2010-09-16T07:50:45Z"));
        assert!(h.contains("<Username>admin</Username>"));
        assert!(
            !h.contains("taadtaadpstcsm"),
            "a senha nunca vai no cabeçalho"
        );
    }

    #[test]
    fn requests_are_well_formed_xml_even_with_awkward_names() {
        let sec = security_header("a&b<c>", "x", &[1, 2, 3], 0);
        for body in [
            BODY_DATE_TIME.to_string(),
            BODY_DEVICE_INFO.into(),
            BODY_CAPABILITIES.into(),
            BODY_PROFILES.into(),
            body_stream_uri("Profile \"1\" & <2>"),
        ] {
            let e = envelope(&sec, &body);
            assert!(roxmltree::Document::parse(&e).is_ok(), "{e}");
        }
    }

    #[test]
    fn base64_handles_every_padding_length() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn profiles_capabilities_and_stream_uri_are_read() {
        let p = parse_profiles(PROFILES);
        assert_eq!(p.len(), 3, "o perfil só de áudio não conta: {p:?}");
        assert_eq!(
            (
                p[0].token.as_str(),
                p[0].width,
                p[0].height,
                p[0].encoding.as_str()
            ),
            ("Profile_1", 1920, 1080, "H264")
        );
        assert_eq!(
            parse_media_xaddr(CAPABILITIES).as_deref(),
            Some("http://192.168.1.46/onvif/media_service")
        );
        assert_eq!(
            parse_stream_uri(STREAM_URI).as_deref(),
            Some("rtsp://192.168.1.46:554/cam/realmonitor?channel=1&subtype=0")
        );
    }

    #[test]
    fn the_main_is_the_biggest_and_the_sub_the_smallest_usable() {
        let (main, sub) = pick_main_and_sub(&parse_profiles(PROFILES)).unwrap();
        assert_eq!(main.token, "Profile_1");
        assert_eq!(sub.unwrap().token, "Profile_2", "o JPEG fica de fora");
        // uma câmera com um perfil só não tem sub
        let one = &parse_profiles(PROFILES)[..1];
        assert!(pick_main_and_sub(one).unwrap().1.is_none());
        assert!(pick_main_and_sub(&[]).is_none());
        // sem H.264/H.265 não há o que cadastrar
        assert!(pick_main_and_sub(&parse_profiles(PROFILES)[2..3]).is_none());
    }

    #[test]
    fn an_authorisation_fault_says_what_to_check() {
        let m = parse_fault(FAULT).unwrap();
        assert!(m.contains("usuário ou senha"), "{m}");
        assert!(parse_fault(STREAM_URI).is_none());
    }

    #[test]
    fn the_secret_placeholder_replaces_the_password_in_the_url() {
        assert_eq!(
            with_secret(
                "rtsp://192.168.1.46:554/cam?channel=1&subtype=0",
                "admin",
                "cam_x_password"
            ),
            "rtsp://admin:${cam_x_password}@192.168.1.46:554/cam?channel=1&subtype=0"
        );
        // credenciais que a câmera tenha posto na URI são trocadas
        assert_eq!(
            with_secret("rtsp://u:senha@h/s", "admin", "seg"),
            "rtsp://admin:${seg}@h/s"
        );
        assert_eq!(
            secret_name("IntelBras 192.168.1.46"),
            "cam_intelbras_192_168_1_46_password"
        );
    }

    #[test]
    fn the_snippet_never_contains_a_password_and_names_the_secret() {
        let found = parse_probe_match(PROBE_MATCH, "x").unwrap();
        let (main, sub) = pick_main_and_sub(&parse_profiles(PROFILES)).unwrap();
        let ins = Inspection {
            manufacturer: "Intelbras".into(),
            model: "iMX-C-309V".into(),
            main: Stream {
                profile: main,
                uri: "rtsp://192.168.1.46:554/cam/realmonitor?channel=1&subtype=0".into(),
            },
            sub: sub.map(|p| Stream {
                profile: p,
                uri: "rtsp://192.168.1.46:554/cam/realmonitor?channel=1&subtype=1".into(),
            }),
        };
        let toml = config_snippet(&found, "admin", &ins);
        assert!(toml.contains("name = \"IntelBras\""), "{toml}");
        assert!(
            toml.contains(
                "url = \"rtsp://admin:${cam_intelbras_192_168_1_46_password}@192.168.1.46:554/"
            ),
            "{toml}"
        );
        assert!(toml.contains("sub_url = ") && toml.contains("subtype=1"));
        // o trecho é um TOML válido para a configuração
        let parsed: crate::config::Config = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.cameras.unwrap().len(), 1);
    }

    #[test]
    fn the_sweep_covers_a_slash_24_but_not_huge_networks() {
        let hosts = subnet_hosts(Ipv4Addr::new(192, 168, 1, 146), 24);
        assert_eq!(hosts.len(), 253, "254 menos o próprio");
        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 46)));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 146)));
        assert!(
            !hosts.contains(&Ipv4Addr::new(192, 168, 1, 0))
                && !hosts.contains(&Ipv4Addr::new(192, 168, 1, 255))
        );
        assert!(subnet_hosts(Ipv4Addr::new(10, 0, 0, 5), 16).is_empty());
        assert!(subnet_hosts(Ipv4Addr::new(10, 0, 0, 5), 32).is_empty());
    }

    #[test]
    fn uuids_look_like_uuids_and_differ() {
        let (a, b) = (new_uuid(), new_uuid());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4', "versão 4");
    }

    #[test]
    fn the_host_is_taken_from_a_url_without_the_credentials() {
        assert_eq!(
            host_of("http://192.168.1.46/onvif/device_service").as_deref(),
            Some("192.168.1.46")
        );
        assert_eq!(
            host_of("http://u:p@cam.local:8080/x").as_deref(),
            Some("cam.local")
        );
        assert!(host_of("sem esquema").is_none());
    }
}
