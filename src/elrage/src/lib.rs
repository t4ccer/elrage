use core::fmt;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    string::FromUtf8Error,
};

use age::{
    decryptor::RecipientsDecryptor, secrecy::SecretString, ssh, stream::StreamReader, x25519,
    Callbacks, Decryptor, Encryptor,
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
                    let error = env.intern(c"user-error");
                    let msg = env.make_string(err.to_string().as_bytes());
                    env.fun_call(error, &[msg])
                }
            }
        }
    };
}

#[derive(Clone)]
struct SshCallbacks {
    env: EmacsEnv,
}

impl Callbacks for SshCallbacks {
    fn display_message(&self, message: &str) {
        let message_fn = self.env.intern(c"message");
        let message_arg = self.env.make_string(message.as_bytes());
        self.env.fun_call(message_fn, &[message_arg]);
    }

    fn confirm(&self, message: &str, _yes_string: &str, _no_string: Option<&str>) -> Option<bool> {
        let y_or_n_p = self.env.intern(c"y-or-n-p");
        let prompt = self.env.make_string(message.as_bytes());
        let value = self.env.fun_call(y_or_n_p, &[prompt]);
        Some(self.env.is_not_nil(value))
    }

    fn request_public_string(&self, description: &str) -> Option<String> {
        let read_minibuffer = self.env.intern(c"read-minibuffer");
        let prompt = self.env.make_string(format!("{description}: ").as_bytes());
        let value = self.env.fun_call(read_minibuffer, &[prompt]);
        let value = self.env.copy_string_to_string(value).ok()?;
        Some(value)
    }

    fn request_passphrase(&self, description: &str) -> Option<SecretString> {
        let read_passwd = self.env.intern(c"read-passwd");
        let prompt = self.env.make_string(format!("{description}: ").as_bytes());
        let password = self.env.fun_call(read_passwd, &[prompt]);
        let password = self.env.copy_string_to_string(password).ok()?;
        Some(SecretString::new(password))
    }
}

#[inline]
fn load_ssh_identity(fp: &str) -> Result<ssh::Identity, ElrageError> {
    let f = BufReader::new(
        File::open(fp).map_err(|err| ElrageError::CouldNotOpenKeyFile(fp.to_string(), err))?,
    );
    let identity = ssh::Identity::from_buffer(f, Some(fp.to_string()))
        .map_err(|err| ElrageError::CouldNotReadKeyFile(fp.to_string(), err))?;
    Ok(identity)
}

#[derive(Debug)]
#[allow(dead_code)]
enum ElrageError {
    AgeEncryptError(age::EncryptError),
    AgeDecryptError(age::DecryptError),
    EncryptedPathNotUtf8(FromUtf8Error),
    CouldNotOpenEncryptedFile(std::io::Error),
    CouldNotOpenKeyFile(String, std::io::Error),
    CouldNotReadKeyFile(String, std::io::Error),
    KeyPathNotUtf8(FromUtf8Error),
    EncryptIoError(std::io::Error),
    DecryptIoError(std::io::Error),
    InvalidRecipient(String),
    NoRecipients,
}

impl fmt::Display for ElrageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElrageError::AgeEncryptError(e) => write!(f, "Age encrypt error: {e}"),
            ElrageError::AgeDecryptError(e) => write!(f, "Age decrypt error: {e}"),
            ElrageError::EncryptedPathNotUtf8(e) => {
                write!(f, "Path to encrypted file is not valid utf8: {e}")
            }
            ElrageError::CouldNotOpenEncryptedFile(e) => {
                write!(f, "Could not open encrypted file: {e}")
            }
            ElrageError::CouldNotOpenKeyFile(fp, e) => {
                write!(f, "Could not open key file '{fp}': {e}")
            }
            ElrageError::CouldNotReadKeyFile(fp, e) => {
                write!(f, "Could not read key file '{fp}': {e}")
            }
            ElrageError::KeyPathNotUtf8(e) => write!(f, "Path to key file is not valid utf8: {e}"),
            ElrageError::EncryptIoError(e) => write!(f, "Age encryption IO error: {e}"),
            ElrageError::DecryptIoError(e) => write!(f, "Age decryption IO error: {e}"),
            ElrageError::InvalidRecipient(e) => write!(f, "Invalid recipient: {e}"),
            ElrageError::NoRecipients => write!(f, "No recipients provided"),
        }
    }
}

impl From<age::DecryptError> for ElrageError {
    fn from(value: age::DecryptError) -> Self {
        Self::AgeDecryptError(value)
    }
}

impl From<age::EncryptError> for ElrageError {
    fn from(value: age::EncryptError) -> Self {
        Self::AgeEncryptError(value)
    }
}

// NOTE: `with_callbacks` returns `impl Identity` and Rust doesn't allow for `impl` in return
// position of generic function trait bounds so we must return whole decrypted streams instead
// of just vectors of recipients without of jumping through hoops of boxing dyn traits objects
// or similar

fn decrypted_stream_from_keys_noninteractive<R>(
    keys: &[String],
    decryptor: RecipientsDecryptor<R>,
) -> Result<StreamReader<R>, ElrageError>
where
    R: std::io::Read,
{
    let identities = keys
        .into_iter()
        .map(|fp| Ok::<_, ElrageError>(load_ssh_identity(fp)?))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(decryptor.decrypt(identities.iter().map(|id| id as &dyn age::Identity))?)
}

fn decrypted_stream_from_keys_interactive<R>(
    env: EmacsEnv,
    keys: &[String],
    decryptor: RecipientsDecryptor<R>,
) -> Result<StreamReader<R>, ElrageError>
where
    R: std::io::Read,
{
    let identities = keys
        .into_iter()
        .map(|fp| Ok::<_, ElrageError>(load_ssh_identity(fp)?.with_callbacks(SshCallbacks { env })))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(decryptor.decrypt(identities.iter().map(|id| id as &dyn age::Identity))?)
}

fn get_string_list(env: EmacsEnv, keys: emacs_value) -> Result<Vec<String>, ElrageError> {
    let length = env.intern(c"length");
    let nth = env.intern(c"nth");
    let keys_len = env.extract_integer(env.fun_call(length, &[keys]));
    let mut key_paths = Vec::with_capacity(keys_len as usize);
    for i in 0..keys_len {
        let i = env.make_integer(i);
        let key = env.fun_call(nth, &[i, keys]);
        key_paths.push(
            env.copy_string_to_string(key)
                .map_err(ElrageError::KeyPathNotUtf8)?,
        );
    }
    Ok(key_paths)
}

fn decrypt_file_worker(
    decrypted_stream_from_keys: impl Fn(
        EmacsEnv,
        &[String],
        RecipientsDecryptor<BufReader<File>>,
    ) -> Result<StreamReader<BufReader<File>>, ElrageError>,
    env: EmacsEnv,
    args: &[emacs_value],
) -> Result<emacs_value, ElrageError> {
    let encrypted_file_path = env
        .copy_string_to_string(args[0])
        .map_err(ElrageError::EncryptedPathNotUtf8)?;
    let encrypted_file = BufReader::new(
        File::open(encrypted_file_path).map_err(ElrageError::CouldNotOpenEncryptedFile)?,
    );

    let key_paths = get_string_list(env, args[1])?;

    let decryptor = match Decryptor::new_buffered(encrypted_file)? {
        Decryptor::Recipients(d) => d,
        Decryptor::Passphrase(_) => unimplemented!(),
    };
    let mut decrypted_stream = decrypted_stream_from_keys(env, &key_paths, decryptor)?;
    let mut decrypted_buffer = Vec::new();
    decrypted_stream
        .read_to_end(&mut decrypted_buffer)
        .map_err(ElrageError::DecryptIoError)?;

    Ok(env.make_string(&decrypted_buffer))
}

fn decrypt_file_interactive(
    env: EmacsEnv,
    args: &[emacs_value],
) -> Result<emacs_value, ElrageError> {
    decrypt_file_worker(decrypted_stream_from_keys_interactive, env, args)
}
wrap_emacs_function!(decrypt_file_interactive_emacs, decrypt_file_interactive);

fn decrypt_file_noninteractive(
    env: EmacsEnv,
    args: &[emacs_value],
) -> Result<emacs_value, ElrageError> {
    decrypt_file_worker(
        |_, keys, decryptor| decrypted_stream_from_keys_noninteractive(keys, decryptor),
        env,
        args,
    )
}
wrap_emacs_function!(
    decrypt_file_noninteractive_emacs,
    decrypt_file_noninteractive
);

fn encrypt_file(env: EmacsEnv, args: &[emacs_value]) -> Result<emacs_value, ElrageError> {
    let mut recipients: Vec<Box<dyn age::Recipient + Send>> = Vec::new();
    for recipient_str in get_string_list(env, args[2])? {
        if let Ok(public_key) = recipient_str.parse::<x25519::Recipient>() {
            recipients.push(Box::new(public_key))
        } else if let Ok(public_key) = recipient_str.parse::<ssh::Recipient>() {
            recipients.push(Box::new(public_key))
        } else {
            return Err(ElrageError::InvalidRecipient(recipient_str));
        }
    }

    let output_file_path = env
        .copy_string_to_string(args[0])
        .map_err(ElrageError::EncryptedPathNotUtf8)?;
    let encryptor = Encryptor::with_recipients(recipients).ok_or(ElrageError::NoRecipients)?;
    let f = BufWriter::new(
        File::create(&output_file_path).map_err(ElrageError::CouldNotOpenEncryptedFile)?,
    );
    let mut f = encryptor.wrap_output(f)?;

    let mut plain_text_buf = Vec::new();
    env.copy_string(args[1], &mut plain_text_buf);

    f.write_all(&plain_text_buf)
        .map_err(ElrageError::EncryptIoError)?;
    f.finish().map_err(ElrageError::EncryptIoError)?;

    Ok(env.intern(c"t"))
}
wrap_emacs_function!(encrypt_file_emacs, encrypt_file);

#[no_mangle]
extern "C" fn emacs_module_init(runtime: *mut emacs_module::internal::emacs_runtime) -> u32 {
    let env = EmacsEnv::from_runtime(runtime);

    env.create_function(
        c"elrage-decrypt-file-interactive",
        2,
        2,
        decrypt_file_interactive_emacs,
        cr#"Decrypt file from FILEPATH using IDENTITIES files.

If to decrypt a key with a passphrase is required it will prompt for passphrase entry.

(fn FILEPATH IDENTITIES)"#,
    );

    env.create_function(
        c"elrage-decrypt-file-noninteractive",
        2,
        2,
        decrypt_file_noninteractive_emacs,
        cr#"Decrypt file from FILEPATH using IDENTITIES files.

If to decrypt a key with a passphrase is required and no other key without passphrase is provided
it will fail to decrypt raising error about no matching keys.
See `elrage-decrypt-file-interactive' for interative version that can prompt for passphrase.


(fn FILEPATH IDENTITIES)"#,
    );

    env.create_function(
        c"elrage-encrypt-file",
        3,
        3,
        encrypt_file_emacs,
        cr#"Decrypt PLAINTEXT using KEYS and save it to FILEPATH

At least one recipient must be present in RECIPIENTS list.

(fn FILEPATH PLAINTEXT RECIPIENTS)"#,
    );

    env.provide(c"elrage");

    return 0;
}
