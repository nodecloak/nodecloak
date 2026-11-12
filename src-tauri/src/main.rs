#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // SAFETY: this is the first operation in main, before Tauri, async runtimes,
    // or any helper threads can read the process environment.
    #[cfg(target_os = "linux")]
    unsafe { nodecloak_lib::sanitize_appimage_environment_before_startup(); }
    let args:Vec<String>=std::env::args().collect();
    #[cfg(windows)]
    if args.get(1).is_some_and(|a|a=="--nodecloak-wsl-relay") {
        let result=if args.len()==3{args[2].parse::<u16>().map_err(|_|"WSL 代理端口无效".to_owned()).and_then(nodecloak_lib::run_wsl_proxy_relay)}else{Err("WSL 代理参数无效".into())};
        if let Err(error)=result{eprintln!("{error}");std::process::exit(1);}return;
    }
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
