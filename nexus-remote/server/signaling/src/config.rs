//! Configuracion del servidor, leida del entorno.
//!
//! Todo se configura por variables de entorno y **nada tiene un valor por
//! omision peligroso**: si falta el secreto de TURN, el servidor no arranca. Es
//! preferible un fallo al arrancar que un servicio en produccion con un secreto
//! vacio.
//!
//! Las variables estan documentadas en `infrastructure/docker/.env.example`.

use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;

/// Fallos de configuracion.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// Falta una variable obligatoria.
    #[error("falta la variable de entorno obligatoria {name}")]
    Missing {
        /// Nombre de la variable.
        name: &'static str,
    },

    /// Una variable tiene un valor que no se puede interpretar.
    #[error("la variable {name} tiene un valor invalido: {value}")]
    Invalid {
        /// Nombre de la variable.
        name: &'static str,
        /// Valor recibido.
        value: String,
    },

    /// Un secreto es demasiado corto para ser seguro.
    #[error("la variable {name} debe tener al menos {min_len} caracteres")]
    SecretTooShort {
        /// Nombre de la variable.
        name: &'static str,
        /// Longitud minima exigida.
        min_len: usize,
    },
}

/// Longitud minima del secreto compartido con Coturn.
///
/// Treinta y dos caracteres aleatorios equivalen a mas de 128 bits de entropia
/// si se generan con `openssl rand -base64 32`, que es lo que indica la guia de
/// despliegue.
pub const MIN_SECRET_LEN: usize = 32;

/// Configuracion completa del servidor de senalizacion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Direccion y puerto de escucha.
    pub bind_address: SocketAddr,
    /// URL publica del servidor, para construir invitaciones de emparejamiento.
    pub public_url: String,
    /// Secreto compartido con Coturn para derivar credenciales temporales.
    pub turn_shared_secret: String,
    /// URL de los servidores TURN que se envian al cliente.
    pub turn_urls: Vec<String>,
    /// URL de los servidores STUN que se envian al cliente.
    pub stun_urls: Vec<String>,
    /// Validez de las credenciales TURN que se emiten.
    pub credential_ttl: Duration,
}

impl Config {
    /// Lee la configuracion del entorno.
    ///
    /// # Errors
    ///
    /// La variante de [`ConfigError`] que corresponda a la primera variable mal
    /// configurada.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Lee la configuracion de una fuente arbitraria.
    ///
    /// Existe para poder probar la validacion sin tocar el entorno del proceso,
    /// que es estado global y haria las pruebas dependientes del orden.
    ///
    /// # Errors
    ///
    /// Igual que [`Self::from_env`].
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&'static str) -> Option<String>,
    {
        let bind_raw = lookup("NEXUS_BIND_ADDRESS").unwrap_or_else(|| "0.0.0.0:8443".to_owned());
        let bind_address = bind_raw.parse().map_err(|_| ConfigError::Invalid {
            name: "NEXUS_BIND_ADDRESS",
            value: bind_raw.clone(),
        })?;

        let public_url = required(&lookup, "NEXUS_PUBLIC_URL")?;

        let turn_shared_secret = required(&lookup, "NEXUS_TURN_SHARED_SECRET")?;
        if turn_shared_secret.len() < MIN_SECRET_LEN {
            return Err(ConfigError::SecretTooShort {
                name: "NEXUS_TURN_SHARED_SECRET",
                min_len: MIN_SECRET_LEN,
            });
        }

        let turn_urls = split_urls(&required(&lookup, "NEXUS_TURN_URLS")?);
        if turn_urls.is_empty() {
            return Err(ConfigError::Invalid {
                name: "NEXUS_TURN_URLS",
                value: "lista vacia".to_owned(),
            });
        }

        let stun_urls = lookup("NEXUS_STUN_URLS")
            .map(|raw| split_urls(&raw))
            .unwrap_or_default();

        let ttl_secs = match lookup("NEXUS_CREDENTIAL_TTL_SECS") {
            Some(raw) => raw.parse::<u64>().map_err(|_| ConfigError::Invalid {
                name: "NEXUS_CREDENTIAL_TTL_SECS",
                value: raw,
            })?,
            None => crate::turn::DEFAULT_CREDENTIAL_TTL.as_secs(),
        };
        if ttl_secs == 0 {
            return Err(ConfigError::Invalid {
                name: "NEXUS_CREDENTIAL_TTL_SECS",
                value: "0".to_owned(),
            });
        }

        Ok(Self {
            bind_address,
            public_url,
            turn_shared_secret,
            turn_urls,
            stun_urls,
            credential_ttl: Duration::from_secs(ttl_secs),
        })
    }
}

fn required<F>(lookup: &F, name: &'static str) -> Result<String, ConfigError>
where
    F: Fn(&'static str) -> Option<String>,
{
    lookup(name)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .ok_or(ConfigError::Missing { name })
}

fn split_urls(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base_env() -> HashMap<&'static str, String> {
        HashMap::from([
            (
                "NEXUS_PUBLIC_URL",
                "wss://nexus.example/signaling".to_owned(),
            ),
            ("NEXUS_TURN_SHARED_SECRET", "a".repeat(MIN_SECRET_LEN)),
            (
                "NEXUS_TURN_URLS",
                "turn:turn.nexus.example:3478?transport=udp".to_owned(),
            ),
        ])
    }

    fn load(env: &HashMap<&'static str, String>) -> Result<Config, ConfigError> {
        Config::from_lookup(|name| env.get(name).cloned())
    }

    #[test]
    fn a_minimal_valid_environment_loads() {
        let config = load(&base_env()).expect("deberia cargar");
        assert_eq!(config.bind_address.port(), 8443);
        assert_eq!(config.turn_urls.len(), 1);
        assert!(config.stun_urls.is_empty());
        assert_eq!(config.credential_ttl, crate::turn::DEFAULT_CREDENTIAL_TTL);
    }

    #[test]
    fn the_public_url_is_mandatory() {
        let mut env = base_env();
        env.remove("NEXUS_PUBLIC_URL");
        assert_eq!(
            load(&env).unwrap_err(),
            ConfigError::Missing {
                name: "NEXUS_PUBLIC_URL"
            }
        );
    }

    #[test]
    fn the_turn_secret_is_mandatory() {
        let mut env = base_env();
        env.remove("NEXUS_TURN_SHARED_SECRET");
        assert_eq!(
            load(&env).unwrap_err(),
            ConfigError::Missing {
                name: "NEXUS_TURN_SHARED_SECRET"
            }
        );
    }

    #[test]
    fn a_short_turn_secret_is_refused() {
        let mut env = base_env();
        env.insert("NEXUS_TURN_SHARED_SECRET", "corto".to_owned());
        assert_eq!(
            load(&env).unwrap_err(),
            ConfigError::SecretTooShort {
                name: "NEXUS_TURN_SHARED_SECRET",
                min_len: MIN_SECRET_LEN
            }
        );
    }

    #[test]
    fn whitespace_only_values_count_as_missing() {
        let mut env = base_env();
        env.insert("NEXUS_PUBLIC_URL", "   ".to_owned());
        assert_eq!(
            load(&env).unwrap_err(),
            ConfigError::Missing {
                name: "NEXUS_PUBLIC_URL"
            }
        );
    }

    #[test]
    fn turn_urls_cannot_be_empty() {
        let mut env = base_env();
        env.insert("NEXUS_TURN_URLS", ",  ,".to_owned());
        assert!(matches!(
            load(&env).unwrap_err(),
            ConfigError::Invalid {
                name: "NEXUS_TURN_URLS",
                ..
            }
        ));
    }

    #[test]
    fn url_lists_are_split_and_trimmed() {
        let mut env = base_env();
        env.insert(
            "NEXUS_TURN_URLS",
            " turn:a:3478 , turns:b:5349 ,".to_owned(),
        );
        env.insert("NEXUS_STUN_URLS", "stun:c:3478,stun:d:3478".to_owned());
        let config = load(&env).expect("carga");
        assert_eq!(config.turn_urls, vec!["turn:a:3478", "turns:b:5349"]);
        assert_eq!(config.stun_urls, vec!["stun:c:3478", "stun:d:3478"]);
    }

    #[test]
    fn a_malformed_bind_address_is_refused() {
        let mut env = base_env();
        env.insert("NEXUS_BIND_ADDRESS", "no-es-una-direccion".to_owned());
        assert!(matches!(
            load(&env).unwrap_err(),
            ConfigError::Invalid {
                name: "NEXUS_BIND_ADDRESS",
                ..
            }
        ));
    }

    #[test]
    fn a_custom_bind_address_is_honoured() {
        let mut env = base_env();
        env.insert("NEXUS_BIND_ADDRESS", "127.0.0.1:9000".to_owned());
        assert_eq!(load(&env).expect("carga").bind_address.port(), 9000);
    }

    #[test]
    fn a_zero_credential_ttl_is_refused() {
        let mut env = base_env();
        env.insert("NEXUS_CREDENTIAL_TTL_SECS", "0".to_owned());
        assert!(matches!(
            load(&env).unwrap_err(),
            ConfigError::Invalid {
                name: "NEXUS_CREDENTIAL_TTL_SECS",
                ..
            }
        ));
    }

    #[test]
    fn a_non_numeric_credential_ttl_is_refused() {
        let mut env = base_env();
        env.insert("NEXUS_CREDENTIAL_TTL_SECS", "doce horas".to_owned());
        assert!(matches!(
            load(&env).unwrap_err(),
            ConfigError::Invalid {
                name: "NEXUS_CREDENTIAL_TTL_SECS",
                ..
            }
        ));
    }

    #[test]
    fn a_custom_credential_ttl_is_honoured() {
        let mut env = base_env();
        env.insert("NEXUS_CREDENTIAL_TTL_SECS", "1800".to_owned());
        assert_eq!(
            load(&env).expect("carga").credential_ttl,
            Duration::from_secs(1_800)
        );
    }
}
