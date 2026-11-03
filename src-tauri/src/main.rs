#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    let args:Vec<String>=std::env::args().collect();
    if args.get(1).is_some_and(|a|a=="--nodecloak-app-proxy") {
        if args.len()!=4{eprintln!("应用代理启动参数无效");std::process::exit(1);}
        let result=args[3].parse::<u32>().map_err(|_|"应用代理进程标识无效".to_owned()).and_then(|parent|nodecloak_lib::run_app_proxy_helper(&args[2],parent));
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}return;
    }
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
