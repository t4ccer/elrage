use std::{
    fs::File,
    io::{BufReader, Read},
    string::FromUtf8Error,
};

use emacs_module::{internal::emacs_value, EmacsEnv};

#[no_mangle]
pub static plugin_is_GPL_compatible: u32 = 0;

macro_rules! wrap_emacs_function {
    ($emacs_fn:ident, $rust_fn: ident) => {
        extern "C" fn $emacs_fn(
            env: *mut emacs_module::internal::emacs_env,
            n_args: isize,
            args: *mut emacs_value,
            _data: *mut std::ffi::c_void,
        ) -> emacs_value {
            let env = EmacsEnv::from_env(env);
            let args = if n_args == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(args, n_args as usize) }
            };

            match $rust_fn(env, args) {
                Ok(v) => v,
                Err(err) => {
                    let error = env.intern(c"error");
                    let msg = env.make_string(format!("{err:?}").as_bytes());
                    env.fun_call(error, &mut [msg])
                }
            }
        }
    };
}

#[derive(Clone)]
struct SshCallbacks {
    env: EmacsEnv,
}

impl age::Callbacks for SshCallbacks {
    fn display_message(&self, message: &str) {
        let message_fn = self.env.intern(c"message");
        let message_arg = self.env.make_string(message.as_bytes());
        self.env.fun_call(message_fn, &mut [message_arg]);
    }

    fn confirm(&self, _message: &str, _yes_string: &str, _no_string: Option<&str>) -> Option<bool> {
        // TODO
        None
    }

    fn request_public_string(&self, description: &str) -> Option<String> {
        let read_minibuffer = self.env.intern(c"read-minibuffer");
        let prompt = self.env.make_string(format!("{description}: ").as_bytes());
        let value = self.env.fun_call(read_minibuffer, &mut [prompt]);
        let value = self.env.copy_string_to_string(value).ok()?;
        Some(value)
    }

    fn request_passphrase(&self, description: &str) -> Option<age::secrecy::SecretString> {
        let read_passwd = self.env.intern(c"read-passwd");
        let prompt = self.env.make_string(format!("{description}: ").as_bytes());
        let password = self.env.fun_call(read_passwd, &mut [prompt]);
        let password = self.env.copy_string_to_string(password).ok()?;
        Some(age::secrecy::SecretString::new(password))
    }
}

#[inline]
fn load_ssh_key_from_file(env: EmacsEnv, fp: &str) -> Result<impl age::Identity, AgenixError> {
    let f = BufReader::new(
        File::open(fp).map_err(|err| AgenixError::CouldNotOpenKeyFile(fp.to_string(), err))?,
    );
    let identity = age::ssh::Identity::from_buffer(f, Some(fp.to_string()))
        .map_err(|err| AgenixError::CouldNotReadKeyFile(fp.to_string(), err))?;
    Ok(identity.with_callbacks(SshCallbacks { env }))
}

#[derive(Debug)]
#[allow(dead_code)]
enum AgenixError {
    Age(age::DecryptError),
    EncryptedPathNotUtf8(FromUtf8Error),
    CouldNotOpenEncryptedFile(std::io::Error),
    CouldNotOpenKeyFile(String, std::io::Error),
    CouldNotReadKeyFile(String, std::io::Error),
    KeyPathNotUtf8(FromUtf8Error),
    DecryptIoError(std::io::Error),
}

impl From<age::DecryptError> for AgenixError {
    fn from(value: age::DecryptError) -> Self {
        Self::Age(value)
    }
}

fn decrypt_file(env: EmacsEnv, args: &[emacs_value]) -> Result<emacs_value, AgenixError> {
    let encrypted_file_path = env
        .copy_string_to_string(args[0])
        .map_err(AgenixError::EncryptedPathNotUtf8)?;
    let encrypted_file = BufReader::new(
        File::open(encrypted_file_path).map_err(AgenixError::CouldNotOpenEncryptedFile)?,
    );
    let decryptor = match age::Decryptor::new_buffered(encrypted_file)? {
        age::Decryptor::Recipients(d) => d,
        age::Decryptor::Passphrase(_) => unimplemented!(),
    };

    let keys = args[1];
    let length = env.intern(c"length");
    let nth = env.intern(c"nth");
    let keys_len = env.extract_integer(env.fun_call(length, &mut [keys]));
    let mut key_paths = Vec::with_capacity(keys_len as usize);
    for i in 0..keys_len {
        let i = env.make_integer(i);
        let key = env.fun_call(nth, &mut [i, keys]);
        key_paths.push(
            env.copy_string_to_string(key)
                .map_err(AgenixError::KeyPathNotUtf8)?,
        );
    }

    let identities = key_paths
        .into_iter()
        .map(|fp| load_ssh_key_from_file(env, &fp))
        .collect::<Result<Vec<_>, _>>()?;

    let mut decrypted_stream =
        decryptor.decrypt(identities.iter().map(|id| id as &dyn age::Identity))?;
    let mut decrypted_buffer = Vec::new();
    decrypted_stream
        .read_to_end(&mut decrypted_buffer)
        .map_err(AgenixError::DecryptIoError)?;

    Ok(env.make_string(&decrypted_buffer))
}
wrap_emacs_function!(decrypt_file_emacs, decrypt_file);

#[no_mangle]
extern "C" fn emacs_module_init(runtime: *mut emacs_module::internal::emacs_runtime) -> u32 {
    let env = EmacsEnv::from_runtime(runtime);

    env.create_function(
        c"agenix-ng-decrypt-file",
        2,
        2,
        decrypt_file_emacs,
        b"Decrypt file from FILEPATH KEYS",
    );

    env.provide(c"agenix-ng");

    return 0;
}
