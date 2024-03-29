use std::{
    sync::{mpsc::Receiver, Arc, Mutex},
    thread::{self, JoinHandle},
};

use crate::block_mod::{block::Block, blockchain::BlockChain, utxo::UnspentTx};

pub fn download_blocks(
    blockchain: Arc<Mutex<BlockChain>>,
    utxo: Arc<Mutex<UnspentTx>>,
    rx: Receiver<Block>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(block) = rx.recv() {
            if let Ok(mut locked_utxo) = utxo.lock() {
                if let Ok(mut locked_blockchain) = blockchain.lock() {
                    locked_utxo.update(&block);
                    locked_blockchain.add(block);

                    if locked_blockchain.cant_blocks() % 1000 == 0 {
                        println!(
                            "Blocks downloadad so far: {}...",
                            locked_blockchain.cant_blocks()
                        );
                    }

                    drop(locked_blockchain);
                }
                drop(locked_utxo);
            }
        }
    })
}
