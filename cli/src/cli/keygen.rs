use error::DefaultResult;
use holochain_common::paths::keys_directory;
use holochain_conductor_api::{
    key_loaders::mock_passphrase_manager,
    keystore::{Keystore, Secret, PRIMARY_KEYBUNDLE_ID},
};
use holochain_dpki::{utils::SeedContext, CONTEXT_SIZE, SEED_SIZE};
use lib3h_sodium::secbuf::SecBuf;
use rpassword;
use std::{
    fs::create_dir_all,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub fn keygen(
    path: Option<PathBuf>,
    passphrase: Option<String>,
    quiet: bool,
    root_seed: Option<String>,
    device_derivation_index: Option<u64>,
) -> DefaultResult<()> {
    let passphrase = passphrase.unwrap_or_else(|| {
        if !quiet {
            println!(
                "
This will create a new agent keystore and populate it with an agent keybundle
containing a public and a private key, for signing and encryption by the agent.
This keybundle will be stored encrypted by passphrase within the keystore file.
The passphrase is securing the keys and will be needed, together with the file,
in order to use the key.
Please enter a secret passphrase below. You will have to enter it again
when unlocking the keybundle to use within a Holochain conductor."
            );
            print!("Passphrase: ");
            io::stdout().flush().expect("Could not flush stdout");
        }
        let passphrase1 = rpassword::read_password().unwrap();
        if !quiet {
            print!("Re-enter passphrase: ");
            io::stdout().flush().expect("Could not flush stdout");
        }
        let passphrase2 = rpassword::read_password().unwrap();
        if passphrase1 != passphrase2 {
            println!("Passphrases do not match. Please retry...");
            ::std::process::exit(1);
        }
        passphrase1
    });

    if !quiet {
        println!("Generating keystore (this will take a few moments)...");
    }

    let (keystore, pub_key) = if root_seed.is_some() {
        let root_seed = root_seed.expect("this to be some as we checked above");
        let device_derivation_index = device_derivation_index.expect(
            "Device derivation context is ensured to be set together with root_seed in main.rs",
        );

        let mut transient_keystore = Keystore::new(mock_passphrase_manager(passphrase.clone()), None)?;

        let mut seed = SecBuf::with_insecure(SEED_SIZE);
        seed.write(0, base64::decode(&root_seed)?.as_slice())
            .expect("SecBuf must be writeable");
        let secret = Arc::new(Mutex::new(Secret::Seed(seed)));

        let mut context_array: [u8; CONTEXT_SIZE] = Default::default();
        let context_string = String::from("HCDEVICE");
        let context_slice = context_string.as_bytes();
        context_array.copy_from_slice(context_slice);
        let context = SeedContext::new(context_array);

        transient_keystore.add("root_seed", secret)?;
        transient_keystore.add_seed_from_seed(
            "root_seed",
            "device_seed",
            &context,
            device_derivation_index,
        )?;

        let mut keystore = Keystore::new(mock_passphrase_manager(passphrase), None)?;
        let device_seed = transient_keystore.get("device_seed")?;
        keystore.add("device_seed", device_seed)?;
        let (pub_key, _) = keystore.add_keybundle_from_seed("device_seed", PRIMARY_KEYBUNDLE_ID)?;
        (keystore, pub_key)
    } else {
        Keystore::new_standalone(mock_passphrase_manager(passphrase), None)?
    };

    let path = if None == path {
        let p = keys_directory();
        create_dir_all(p.clone())?;
        p.join(pub_key.clone())
    } else {
        path.unwrap()
    };

    keystore.save(path.clone())?;
    let path_str = path.to_str().unwrap();

    if quiet {
        println!("{}", pub_key);
        println!("{}", path_str);
    } else {
        println!("");
        println!("Succesfully created new agent keystore.");
        println!("");
        println!("Public address: {}", pub_key);
        println!("Keystore written to: {}", path_str);
        println!("");
        println!("You can set this file in a conductor config as keystore_file for an agent.");
    }
    Ok(())
}

#[cfg(test)]
pub mod test {
    use super::*;
    use holochain_conductor_api::{
        key_loaders::mock_passphrase_manager,
        keystore::{Keystore, PRIMARY_KEYBUNDLE_ID},
    };
    use std::{fs::remove_file, path::PathBuf};

    #[test]
    fn keygen_roundtrip() {
        let path = PathBuf::new().join("test.key");
        let passphrase = String::from("secret");

        keygen(
            Some(path.clone()),
            Some(passphrase.clone()),
            true,
            None,
            None
        ).expect("Keygen should work");

        let mut keystore =
            Keystore::new_from_file(path.clone(), mock_passphrase_manager(passphrase), None)
                .unwrap();

        let keybundle = keystore.get_keybundle(PRIMARY_KEYBUNDLE_ID);

        assert!(keybundle.is_ok());

        let _ = remove_file(path);
    }
}
