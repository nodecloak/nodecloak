#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    let args:Vec<String>=std::env::args().collect();
    if args.len()==2&&args[1]=="--nodecloak-proxy-core" {
        if let Err(error)=nodecloak_lib::run_proxy_core_stdio(){eprintln!("{error}");std::process::exit(1);}
        return;
    }
    if nodecloak_lib::firefox_native_host::is_invocation(&args) {
        if let Err(error)=nodecloak_lib::firefox_native_host::run_stdio() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a|a=="--mcp"){
        let value=|flag:&str|args.iter().position(|a|a==flag).and_then(|i|args.get(i+1)).cloned().unwrap_or_default();
        if let Err(e)=nodecloak_lib::mcp::run_stdio(value("--instance"),value("--client")){eprintln!("{e}");std::process::exit(1);}
        return;
    }
    nodecloak_lib::run();
}
