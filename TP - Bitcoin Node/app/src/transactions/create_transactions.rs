use bitcoin::block_mod::{script::Script, transaction::Transaction, tx_out::TxOut, tx_in::TxIn};
use bitcoin_hashes::{hash160, sha256, Hash, sha256d};
use secp256k1::{SecretKey, Secp256k1, PublicKey, Message};
use bech32::wit_prog::WitnessProgram;

use super::create_transaction_error::TransactionCreateError;


pub fn is_string_bech32(address: String) -> bool{
    WitnessProgram::from_address("tb".to_string(), address).is_ok()
}

fn is_array_bech32(address: &[u8]) -> bool{
    is_string_bech32(String::from_utf8_lossy(address).to_string())
}

fn address_from_pubkey(public_key: &[u8], p2wpkh: bool) -> Vec<u8>{
    let h160 = hash160::Hash::hash(public_key).to_byte_array();

    if p2wpkh{
        let witness_program = WitnessProgram{
            version: 0,
            program: h160.to_vec(),
        };
    
        return witness_program.to_address("tb".to_string()).unwrap().as_bytes().to_vec();
    }

    let version_prefix: [u8; 1] = [0x6f];
    let double_hash = sha256d::Hash::hash(&[&version_prefix[..], &h160[..]].concat());    
    let checksum = &double_hash[..4];
    
    let input = [&version_prefix[..], &h160[..], checksum].concat();

    bs58::encode(input).into_vec()
}

fn decode_base58(address: &Vec<u8>) -> Vec<u8> {
    if let Ok(combined) = bs58::decode(address).into_vec(){
        return combined[1..combined.len() - 4].to_vec();
    };

    Vec::new()
}

pub fn pk_script_from_pubkey(public_key: &[u8], p2wpkh: bool) -> Vec<u8> {
    let address = address_from_pubkey(public_key, p2wpkh);

    pk_script_from_address(&address, p2wpkh)
}

pub fn pk_script_from_address(address: &Vec<u8>, p2wpkh: bool) -> Vec<u8>{
    if p2wpkh{
        let string_address = String::from_utf8_lossy(address).to_string(); 

        if let Ok(witness_program) = WitnessProgram::from_address("tb".to_string(), string_address){
            return witness_program.to_scriptpubkey();
        }
    }

    let h160 = decode_base58(address);
    let script = Script::new(Some(vec![vec![0x76], vec![0xa9], h160, vec![0x88], vec![0xac]]));
    script.as_bytes()
}

fn create_txout_list(targets: Vec<(Vec<u8>, i64)>, fee: i64) -> (Vec<TxOut>, i64){
    let mut total_amount = fee;
    let mut txout_list = vec![];

    for (address, amount) in targets {
        let script = pk_script_from_address(&address, is_array_bech32(&address));
        let txout = TxOut::new(amount, script/*.as_bytes()*/);
        
        total_amount += amount;
        txout_list.push(txout);
    }
    (txout_list, total_amount)
}

fn create_txin_list(mut utxo: Vec<(Vec<u8>, u32, TxOut)>, total_amount: i64) -> Result<(Vec<TxIn>, Vec<i64>), TransactionCreateError> {
    let mut txin_list = vec![];
    let mut amount_list = vec![];
    let mut acum_amount = 0;

    while acum_amount < total_amount {
        if let Some(txout) = utxo.pop() {
            let txin = TxIn::new(txout.0, txout.1, vec![], 0xffffffff);

            acum_amount += txout.2.get_value();

            txin_list.push(txin);
            amount_list.push(txout.2.get_value());
        } else {
            return Err(TransactionCreateError::InsufficientFounds);
        }
    }

    amount_list.push(acum_amount - total_amount);   //Change difference that must return to the sender

    Ok((txin_list, amount_list))
}



fn sign_transaction(transaction: &mut Transaction, private_key: SecretKey, pk_script: &[u8], p2wpkh: bool, amount_list: &[i64]){
    let secp = Secp256k1::new();
    let mut signature_hash;

    for i in 0..transaction.get_tx_in_list().len(){
        if p2wpkh{
            signature_hash = transaction.p2wpkh_signature_hash(i, pk_script.to_vec(), amount_list.to_vec());
        } else{
            signature_hash = transaction.p2pkh_signature_hash(i, pk_script);
        }
        
        let message = Message::from_hashed_data::<sha256::Hash>(&signature_hash);        
        let mut signature = secp.sign_ecdsa(&message, &private_key).serialize_der().to_vec();
        signature.push(0x01);

        let pubkey = PublicKey::from_secret_key(&secp, &private_key).serialize().to_vec();
        let script = vec![signature, pubkey];

        if p2wpkh{
            transaction.set_witness(script);
        } else{
            let signature_script = Script::new(Some(script));    
            transaction.set_signature(i, signature_script.as_bytes());
        }
    }
}


pub fn create_transaction(targets: Vec<(Vec<u8>, i64)>, utxo: Vec<(Vec<u8>, u32, TxOut)>, private_key: &[u8], fee: i64, p2wpkh: bool) -> Result<Transaction, TransactionCreateError> {
    let secp = Secp256k1::new();

    let private_key = SecretKey::from_slice(private_key).map_err(|_| TransactionCreateError::PrivateKey)?;
    let public_key = PublicKey::from_secret_key(&secp, &private_key).serialize().to_vec();
    let pk_script = pk_script_from_pubkey(&public_key, p2wpkh);

    let (mut txout_list, total_amount) = create_txout_list(targets, fee);
    let (txin_list, amount_list)= create_txin_list(utxo, total_amount)?;

    if let Some(change) = amount_list.last(){
        let txout_change = TxOut::new(*change, pk_script.clone());
        txout_list.push(txout_change);
    }

    let mut transaction = Transaction::new(1, txin_list, txout_list, 0);

    sign_transaction(&mut transaction, private_key, &pk_script, p2wpkh, &amount_list);

    Ok(transaction)
}

#[cfg(test)]
mod create_transactions_test {
    use std::str::FromStr;

    use bitcoin::{messages::{read_from_bytes::{decode_hex, encode_hex}, compact_size::CompactSizeUInt}, block_mod::{tx_in::TxIn, tx_out::TxOut, script::Script, transaction::Transaction}};
    use bitcoin_hashes::*;
    use secp256k1::{Secp256k1, Message, SecretKey, PublicKey};

    use crate::transactions::{create_transactions::{decode_base58, is_string_bech32, address_from_pubkey, is_array_bech32}, create_transaction_error::TransactionCreateError};

    use super::{pk_script_from_address};

    #[test]
    pub fn create_transaction() -> Result<(), TransactionCreateError>{
        // total de la txout que voy a usar 0.01875221
        // quiero depositar desde la cuenta address a la cuenta target 0.012345

        let address = b"n1mDu5Zd5qS75vqK1yqnKmEZQzDyncQqj4".to_vec();
        let target = b"mp3PDnKDtxPYrPKcYLGX1pXMe6KwAsfquD".to_vec();

        //let pub_key = "02E641B11A0FB5A761814D0F166ADC4E654037C844B44226219AE3D6947EBC4DA6";
        let private_key = "740A9C5D2BD171E99DDDC268A26179FCAD9BFE9A7A8188725EDA0D1D9F6D2264";

        let mut prev_tx = decode_hex("7a56640d6c89ce4744ab77c5332c87fec02c58720a7fc1ba19d6b6546f5b29e8")?;
        prev_tx.reverse();
        let prev_index = 0; // 0.01875221 -0.012345 - 0.003

        let txin = TxIn::new(prev_tx, prev_index, vec![], 0xffffffff);

        // calculo el cambio
        let change_amount = 0.0009 * 100000000.0;
        let change_h160 = decode_base58(&address);
        let change_script = Script::new(Some(vec![vec![0x76], vec![0xa9], change_h160, vec![0x88], vec![0xac]]));
        let change_txout = TxOut::new(change_amount as i64, change_script.as_bytes());


        let target_amount = 0.0021 * 100000000.0;
        let target_h160 = decode_base58(&target);
        let target_script = Script::new(Some(vec![vec![0x76], vec![0xa9], target_h160, vec![0x88], vec![0xac]]));
        let target_txout = TxOut::new(target_amount as i64, target_script.as_bytes());

        let mut tx = Transaction::new(1, vec![txin], vec![change_txout, target_txout], 0);

        let secp = Secp256k1::new();

        let signature_hash = tx.p2pkh_signature_hash(0, &change_script.as_bytes());

        let private_key = SecretKey::from_str(private_key)?;

        let message = Message::from_hashed_data::<sha256::Hash>(&signature_hash);

        let der = secp.sign_ecdsa(&message, &private_key).serialize_der().to_vec();

        let sig = vec![der, vec![1_u8]].concat();

        let sec = PublicKey::from_secret_key(&secp, &private_key).serialize().to_vec();

        let signature_script = Script::new(Some(vec![sig, sec]));

        tx.set_signature(0, signature_script.as_bytes());

        println!("{:?}", encode_hex(&tx.as_bytes(false))?);

        Ok(())
    }
    #[test]
    pub fn test_address_from_public_key() -> Result<(), TransactionCreateError>{
        let public_key = decode_hex("02E641B11A0FB5A761814D0F166ADC4E654037C844B44226219AE3D6947EBC4DA6")?;
        let address = b"n1mDu5Zd5qS75vqK1yqnKmEZQzDyncQqj4".to_vec();

        let address_calculated = address_from_pubkey(&public_key, false);

        assert_eq!(address_calculated, address);
        Ok(())
    }

    #[test]
    pub fn test_pk_script_from_address() -> Result<(), TransactionCreateError>{
        let address = b"mzx5YhAH9kNHtcN481u6WkjeHjYtVeKVh2".to_vec(); //ejemplo sacado del libro

        let h160 = decode_hex("d52ad7ca9b3d096a38e752c2018e6fbc40cdf26f")?;
        let pk_script = vec![vec![0x76], vec![0xa9], vec![20], h160, vec![0x88], vec![0xac]].concat();
        
        let pk_script_calculated = pk_script_from_address(&address, false);

        assert_eq!(pk_script, pk_script_calculated);

        Ok(())
    }

    #[test]
    pub fn test_pk_script_from_pubkey() -> Result<(), TransactionCreateError>{
        let public_key = decode_hex("0362599B444272856B51E7EE10A4B70A683A9965AD3859E4D75E9B9EC136F84144")?;

        println!("{}", public_key.len());
        let address = address_from_pubkey(&public_key, false);

        let pk_script = pk_script_from_address(&address, false);

        println!("{:?}", pk_script);

        Ok(())
    }

    #[test]
    pub fn test_create_transaction_2() -> Result<(), TransactionCreateError> {
        let address = b"mq5boK8wasubp4QHZ349damhWQLCthdrKP".to_vec();
        let target = b"n3yL92bzbMkicfYwUS3K7huHj81ew877ob".to_vec();

        let private_key = "11063638E1C47A9EEEDCDB476654644B00F7BFF9798031CFBB1EB9DA4D8B51F4";

        let mut prev_tx = decode_hex("3464a5386b818c901b910d96ee71bce0ea9a4465719ea458deea9df81e8504f5")?;
        prev_tx.reverse();
        let prev_index = 0;

        let txin = TxIn::new(prev_tx, prev_index, vec![], 0xffffffff);

        // calculo el cambio
        let change_amount = 0.0009 * 100000000.0;
        let change_h160 = decode_base58(&address);
        let change_script = Script::new(Some(vec![vec![0x76], vec![0xa9], change_h160, vec![0x88], vec![0xac]]));
        let change_txout = TxOut::new(change_amount as i64, change_script.as_bytes());


        let target_amount = 0.0021 * 100000000.0;
        let target_h160 = decode_base58(&target);
        let target_script = Script::new(Some(vec![vec![0x76], vec![0xa9], target_h160, vec![0x88], vec![0xac]]));
        let target_txout = TxOut::new(target_amount as i64, target_script.as_bytes());

        let mut tx = Transaction::new(1, vec![txin], vec![change_txout, target_txout], 0);

        let secp = Secp256k1::new();

        let signature_hash = tx.p2pkh_signature_hash(0, &change_script.as_bytes());

        let private_key = SecretKey::from_str(private_key)?;

        let message = Message::from_hashed_data::<sha256::Hash>(&signature_hash);

        let der = secp.sign_ecdsa(&message, &private_key).serialize_der().to_vec();

        let sig = vec![der, vec![1_u8]].concat();
        println!("len sig {}", sig.len());

        let sec = PublicKey::from_secret_key(&secp, &private_key).serialize().to_vec();

        println!("len sec {}", sec.len());

        let signature_script = Script::new(Some(vec![sig, sec]));

        tx.set_signature(0, signature_script.as_bytes());

        println!("len script {}", signature_script.as_bytes().len());

        println!("{:?}", encode_hex(&tx.as_bytes(false))?);

        Ok(())
    }

    #[test]
    pub fn test_is_bech32() {
        assert!(is_string_bech32("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".to_string()));
        assert!(!is_string_bech32("tb1qw308d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".to_string()));
    }
    
    #[test]
    pub fn test_pk_script() {
        let address = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".to_string();
        let address2 = b"mq5boK8wasubp4QHZ349damhWQLCthdrKP".to_vec();

        assert_eq!(pk_script_from_address(&address.as_bytes().to_vec(), true), [0, 20, 117, 30, 118, 232, 25, 145, 150, 212, 84, 148, 28, 69, 209, 179, 163, 35, 241, 67, 59, 214]);
        assert_eq!(pk_script_from_address(&address2, true), pk_script_from_address(&address2, false));

        let asas = CompactSizeUInt::from_number([117, 30, 118, 232, 25, 145, 150, 212, 84, 148, 28, 69, 209, 179, 163, 35, 241, 67, 59, 214].len().try_into().unwrap());

        println!("{:?}", asas.as_bytes());
    }

    #[test]
    pub fn test_pk_to_address() -> Result<(), TransactionCreateError>{
        let public_key1 = decode_hex("0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798")?;
        let public_key2 = decode_hex("FEABBD73DDD97F2C00CC6023B38C08214736CAF26A8ED91CE1ABA30D8BE46B35")?;

        let address1 = address_from_pubkey(&public_key1, true);
        let address2 = address_from_pubkey(&public_key2, false);

        assert_eq!(address1, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx".to_string().as_bytes());
        assert!(is_array_bech32(&address1));
        assert!(!is_array_bech32(&address2));

        Ok(())
    }
    
    #[test]
    pub fn test_signature() -> Result<(), TransactionCreateError>{
        let h160: Vec<u8> = vec![0, 20, 117, 30, 118, 232, 25, 145, 150, 212, 84, 148, 28, 69, 209, 179, 163, 35, 241, 67, 59, 214];
        let private_key = decode_hex("11063638E1C47A9EEEDCDB476654644B00F7BFF9798031CFBB1EB9DA4D8B51F4")?;
        let private_key = SecretKey::from_slice(&private_key).map_err(|_| TransactionCreateError::PrivateKey)?;

        let secp = Secp256k1::new();

        let message = Message::from_hashed_data::<sha256::Hash>(&h160);
        let der = secp.sign_ecdsa(&message, &private_key).serialize_der().to_vec();
        let sig = vec![der, vec![1_u8]].concat();
        let sec = PublicKey::from_secret_key(&secp, &private_key).serialize().to_vec();

        println!("{:?}", sig);
        println!("{:?}", sec);

        Ok(())
    }
}