# Segurança

## Credenciais

URLs de câmera costumam conter usuário e senha (`rtsp://usuario:senha@host`).

- Mantenha o `config.toml` **fora do controle de versão** — ele já está no
  `.gitignore`. Use `config.toml.example` apenas com valores fictícios.
- O aplicativo mascara senhas em todos os logs (`domain::redact::mask_credentials`);
  as zonas de movimento são salvas por **nome da câmera**, nunca pela URL.
- Se uma senha de câmera chegar a um repositório público, troque-a na câmera:
  remover o commit não a torna secreta de novo.

## Reportar uma vulnerabilidade

Abra um *security advisory* privado em
<https://github.com/cesarfilho/rust-rtsp-viewer/security/advisories/new> em
vez de uma issue pública. Inclua a versão (`Cargo.toml`), o sistema e os passos
para reproduzir.
