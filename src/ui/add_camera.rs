//! O assistente "Adicionar câmera" (plano D1): procura câmeras ONVIF na rede, pede usuário e senha,
//! lê os streams, guarda a senha no chaveiro e entrega o trecho do `config.toml` para colar.
//!
//! O estado e as transições são **puros** (testados sem janela): [`AddCamera::apply`] recebe uma
//! mensagem e devolve o que a janela deve fazer ([`Effect`]: buscar, ler, copiar, fechar). A rede e o
//! chaveiro rodam numa thread à parte (`update::add_camera_tasks`); a senha só existe no campo de
//! texto e na chamada de leitura, **nunca no log, no `Debug` nem no trecho gerado**.

use crate::i18n::{t, tf};
use crate::onvif::Found;

/// O que a leitura de uma câmera devolve para a tela.
#[derive(Clone, PartialEq, Eq)]
pub struct Inspected {
    /// O trecho de `[[cameras]]`, com `${segredo}` no lugar da senha.
    pub snippet: String,
    /// O nome do segredo que o trecho usa.
    pub secret: String,
    /// A senha foi para o chaveiro?
    pub stored: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Stage {
    Scanning,
    Pick {
        found: Vec<Found>,
    },
    Credentials {
        /// A lista de onde veio, para o "Voltar".
        list: Vec<Found>,
        found: Found,
        user: String,
        password: String,
        error: Option<String>,
    },
    Inspecting {
        found: Found,
        user: String,
    },
    Done(Inspected),
}

#[derive(Clone, PartialEq, Eq)]
pub struct AddCamera {
    pub stage: Stage,
}

#[derive(Clone)]
pub enum AddMsg {
    /// A varredura terminou.
    Scanned(Vec<Found>),
    Pick(usize),
    User(String),
    Password(String),
    Submit,
    Inspected(Result<Inspected, String>),
    CopySnippet,
    Rescan,
    /// Volta da tela de senha para a lista.
    Back,
    Close,
}

/// O que a janela faz depois de uma transição.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    Scan,
    /// Ler os streams: `(câmera, usuário, senha)`. A senha sai do estado aqui e não volta.
    Inspect(Found, String, String),
    Copy(String),
    Close,
}

// `Debug` à mão: a senha nunca aparece, nem num `{:?}` esquecido.
impl std::fmt::Debug for AddMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddMsg::Password(_) => f.write_str("Password(****)"),
            AddMsg::Scanned(v) => write!(f, "Scanned({})", v.len()),
            AddMsg::Pick(i) => write!(f, "Pick({i})"),
            AddMsg::User(u) => write!(f, "User({u})"),
            AddMsg::Submit => f.write_str("Submit"),
            AddMsg::Inspected(r) => write!(f, "Inspected(ok={})", r.is_ok()),
            AddMsg::CopySnippet => f.write_str("CopySnippet"),
            AddMsg::Rescan => f.write_str("Rescan"),
            AddMsg::Back => f.write_str("Back"),
            AddMsg::Close => f.write_str("Close"),
        }
    }
}

impl std::fmt::Debug for AddCamera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.stage {
            Stage::Scanning => f.write_str("AddCamera(Scanning)"),
            Stage::Pick { .. } => f.write_str("AddCamera(Pick)"),
            Stage::Credentials { .. } => f.write_str("AddCamera(Credentials)"),
            Stage::Inspecting { .. } => f.write_str("AddCamera(Inspecting)"),
            Stage::Done(_) => f.write_str("AddCamera(Done)"),
        }
    }
}

impl AddCamera {
    /// Abre o assistente já procurando.
    pub fn open() -> (Self, Effect) {
        (
            Self {
                stage: Stage::Scanning,
            },
            Effect::Scan,
        )
    }

    pub fn apply(&mut self, msg: AddMsg) -> Effect {
        match (&mut self.stage, msg) {
            (_, AddMsg::Close) => Effect::Close,
            (_, AddMsg::Rescan) => {
                self.stage = Stage::Scanning;
                Effect::Scan
            }
            (Stage::Scanning, AddMsg::Scanned(found)) => {
                self.stage = Stage::Pick { found };
                Effect::None
            }
            (Stage::Pick { found }, AddMsg::Pick(i)) => {
                if let Some(f) = found.get(i).cloned() {
                    self.stage = Stage::Credentials {
                        list: std::mem::take(found),
                        found: f,
                        user: "admin".to_string(),
                        password: String::new(),
                        error: None,
                    };
                }
                Effect::None
            }
            (Stage::Credentials { user, error, .. }, AddMsg::User(u)) => {
                *user = u;
                *error = None;
                Effect::None
            }
            (
                Stage::Credentials {
                    password, error, ..
                },
                AddMsg::Password(p),
            ) => {
                *password = p;
                *error = None;
                Effect::None
            }
            (Stage::Credentials { list, .. }, AddMsg::Back) => {
                self.stage = Stage::Pick {
                    found: std::mem::take(list),
                };
                Effect::None
            }
            (
                Stage::Credentials {
                    found,
                    user,
                    password,
                    error,
                    ..
                },
                AddMsg::Submit,
            ) => {
                if user.trim().is_empty() || password.is_empty() {
                    *error = Some(t("Informe o usuário e a senha").to_string());
                    return Effect::None;
                }
                let (found, user, password) = (
                    found.clone(),
                    user.trim().to_string(),
                    std::mem::take(password),
                );
                self.stage = Stage::Inspecting {
                    found: found.clone(),
                    user: user.clone(),
                };
                Effect::Inspect(found, user, password)
            }
            (Stage::Inspecting { found, user }, AddMsg::Inspected(result)) => {
                match result {
                    Ok(done) => self.stage = Stage::Done(done),
                    Err(message) => {
                        self.stage = Stage::Credentials {
                            list: Vec::new(),
                            found: found.clone(),
                            user: user.clone(),
                            password: String::new(),
                            error: Some(message),
                        };
                    }
                }
                Effect::None
            }
            (Stage::Done(done), AddMsg::CopySnippet) => Effect::Copy(done.snippet.clone()),
            // qualquer outra combinação (uma resposta atrasada, um clique fora de hora) não faz nada
            _ => Effect::None,
        }
    }

    /// A descrição de uma câmera na lista: `192.168.1.46 · IntelBras iMX-C-309V`.
    pub fn describe(f: &Found) -> String {
        let name = f.name.as_deref().unwrap_or("");
        let hw = f.hardware.as_deref().unwrap_or("");
        let what = format!("{name} {hw}");
        let what = what.trim();
        if what.is_empty() {
            f.ip.clone()
        } else {
            format!("{} · {what}", f.ip)
        }
    }

    /// A frase sobre o chaveiro na tela final.
    pub fn keyring_note(done: &Inspected) -> String {
        if done.stored {
            tf("Senha guardada no chaveiro como `{}`.", &[&done.secret])
        } else {
            tf(
                "Não consegui guardar a senha no chaveiro. Guarde com: rrvctl secret set {}",
                &[&done.secret],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(ip: &str) -> Found {
        Found {
            xaddr: format!("http://{ip}/onvif/device_service"),
            ip: ip.into(),
            name: Some("IntelBras".into()),
            hardware: Some("iMX-C-309V".into()),
        }
    }

    fn done() -> Inspected {
        Inspected {
            snippet: "[[cameras]]\nname = \"x\"\n".into(),
            secret: "cam_x_password".into(),
            stored: true,
        }
    }

    fn at_credentials() -> AddCamera {
        let (mut a, _) = AddCamera::open();
        a.apply(AddMsg::Scanned(vec![cam("10.0.0.5"), cam("10.0.0.6")]));
        a.apply(AddMsg::Pick(1));
        a
    }

    #[test]
    fn opening_starts_a_scan() {
        let (a, fx) = AddCamera::open();
        assert_eq!(fx, Effect::Scan);
        assert!(matches!(a.stage, Stage::Scanning));
    }

    #[test]
    fn the_scan_result_becomes_the_list_and_an_empty_one_is_still_a_list() {
        let (mut a, _) = AddCamera::open();
        assert_eq!(a.apply(AddMsg::Scanned(vec![])), Effect::None);
        assert!(matches!(&a.stage, Stage::Pick { found } if found.is_empty()));
    }

    #[test]
    fn picking_a_camera_asks_for_credentials_with_admin_as_the_usual_user() {
        let a = at_credentials();
        let Stage::Credentials {
            found,
            user,
            password,
            error,
            ..
        } = &a.stage
        else {
            panic!("{a:?}");
        };
        assert_eq!(found.ip, "10.0.0.6", "a escolhida, não a primeira");
        assert_eq!(
            (user.as_str(), password.as_str(), error.as_ref()),
            ("admin", "", None)
        );
    }

    #[test]
    fn an_empty_user_or_password_is_refused_without_touching_the_network() {
        let mut a = at_credentials();
        assert_eq!(a.apply(AddMsg::Submit), Effect::None, "sem senha");
        let Stage::Credentials { error, .. } = &a.stage else {
            panic!()
        };
        assert!(error.is_some());
        a.apply(AddMsg::Password("s".into()));
        a.apply(AddMsg::User("  ".into()));
        assert_eq!(a.apply(AddMsg::Submit), Effect::None, "sem usuário");
    }

    #[test]
    fn submitting_hands_the_password_to_the_task_and_forgets_it() {
        let mut a = at_credentials();
        a.apply(AddMsg::User(" root ".into()));
        a.apply(AddMsg::Password("segredo".into()));
        let fx = a.apply(AddMsg::Submit);
        let Effect::Inspect(f, user, pass) = fx else {
            panic!("{fx:?}")
        };
        assert_eq!(
            (f.ip.as_str(), user.as_str(), pass.as_str()),
            ("10.0.0.6", "root", "segredo")
        );
        assert!(matches!(a.stage, Stage::Inspecting { .. }));
        // nada no estado nem no Debug guarda a senha
        assert!(!format!("{a:?}").contains("segredo"));
        assert!(!format!("{:?}", AddMsg::Password("segredo".into())).contains("segredo"));
    }

    #[test]
    fn a_wrong_password_returns_to_the_form_with_the_reason_and_an_empty_password() {
        let mut a = at_credentials();
        a.apply(AddMsg::Password("errada".into()));
        a.apply(AddMsg::Submit);
        a.apply(AddMsg::Inspected(Err(
            "usuário ou senha recusados pela câmera".into(),
        )));
        let Stage::Credentials {
            found,
            user,
            password,
            error,
            ..
        } = &a.stage
        else {
            panic!("{a:?}");
        };
        assert_eq!(
            (found.ip.as_str(), user.as_str(), password.as_str()),
            ("10.0.0.6", "admin", "")
        );
        assert!(error.as_deref().unwrap().contains("recusados"));
        // digitar de novo limpa o erro
        a.apply(AddMsg::Password("x".into()));
        let Stage::Credentials { error, .. } = &a.stage else {
            panic!()
        };
        assert!(error.is_none());
    }

    #[test]
    fn success_shows_the_snippet_and_copy_hands_it_to_the_clipboard() {
        let mut a = at_credentials();
        a.apply(AddMsg::Password("ok".into()));
        a.apply(AddMsg::Submit);
        a.apply(AddMsg::Inspected(Ok(done())));
        assert!(matches!(a.stage, Stage::Done(_)));
        assert_eq!(a.apply(AddMsg::CopySnippet), Effect::Copy(done().snippet));
    }

    #[test]
    fn close_and_rescan_work_from_anywhere_and_late_replies_are_ignored() {
        let mut a = at_credentials();
        assert_eq!(a.apply(AddMsg::Rescan), Effect::Scan);
        assert!(matches!(a.stage, Stage::Scanning));
        // uma leitura que chega depois de voltar à busca não mexe em nada
        assert_eq!(a.apply(AddMsg::Inspected(Ok(done()))), Effect::None);
        assert!(matches!(a.stage, Stage::Scanning));
        assert_eq!(a.apply(AddMsg::Close), Effect::Close);
        // "Voltar" devolve a lista que já estava na mão, sem procurar de novo
        let mut b = at_credentials();
        assert_eq!(b.apply(AddMsg::Back), Effect::None);
        assert!(matches!(&b.stage, Stage::Pick { found } if found.len() == 2));
    }

    #[test]
    fn the_list_line_names_the_address_and_what_the_camera_says_it_is() {
        assert_eq!(
            AddCamera::describe(&cam("10.0.0.5")),
            "10.0.0.5 · IntelBras iMX-C-309V"
        );
        let bare = Found {
            xaddr: "http://1.2.3.4/x".into(),
            ip: "1.2.3.4".into(),
            name: None,
            hardware: None,
        };
        assert_eq!(AddCamera::describe(&bare), "1.2.3.4");
    }

    #[test]
    fn the_keyring_note_tells_what_to_do_when_the_password_was_not_stored() {
        let mut d = done();
        assert!(AddCamera::keyring_note(&d).contains("cam_x_password"));
        d.stored = false;
        assert!(AddCamera::keyring_note(&d).contains("rrvctl secret set cam_x_password"));
    }
}
