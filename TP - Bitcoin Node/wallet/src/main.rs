use glib::MainContext;
use glib::Priority;
use glib::Type;
use gtk::Builder;
use gtk::ListStore;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use wallet::accounts::Accounts;
use wallet::handlers::handle_buttons::set_buttons;
use wallet::handlers::handle_windows::set_windows;
use wallet::transactions::transaction_view::update_transaction_list;
use wallet::update_wallet::update_wallet;

fn main() {
    let socket: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8000);
    let node: Arc<Mutex<TcpStream>> = match TcpStream::connect(socket) {
        Ok(conexion) => Arc::new(Mutex::new(conexion)),
        Err(_) => {
            println!("Failed to connect to node.");
            return;
        }
    };

    let accounts: Arc<Mutex<Accounts>> = Arc::new(Mutex::new(Accounts::new()));

    let (tx_sender, tx_recv): (glib::Sender<bool>, glib::Receiver<bool>) =
        MainContext::channel(Priority::default());
    let shared_accounts = accounts.clone();
    let shared_node = node.clone();

    let handle_interface = thread::spawn(move || {
        if let Err(err) = gtk::init() {
            eprintln!("Failed to initialize GTK: {}", err);
            return;
        }

        let store = ListStore::new(&[
            Type::String,
            Type::String,
            Type::String,
            Type::String,
            Type::String,
        ]);

        let glade_src = include_str!("../bitcoin_ui.glade");
        let builder = Builder::from_string(glade_src);

        if let Err(err) = set_windows(&builder) {
            println!("{:?}", err);
        };

        if let Err(err) = set_buttons(&builder, shared_accounts.clone(), shared_node, &store) {
            println!("{:?}", err);
        };

        if let Err(err) = update_transaction_list(&builder, store, shared_accounts.clone(), tx_recv)
        {
            println!("{:?}", err);
        };

        gtk::main();
    });

    if let Err(err) = update_wallet(accounts, node, tx_sender) {
        println!("{:?}", err);
    };
    if let Err(err) = handle_interface.join() {
        println!("{:?}", err);
    };
}
