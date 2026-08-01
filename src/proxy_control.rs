// src/proxy_control.rs
// 代理控制，用以解决从设备开启关闭代理的问题

use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

// 保存原始代理状态（用于退出时恢复）
static ORIGINAL_PROXY_STATE: std::sync::Mutex<Option<ProxyState>> = std::sync::Mutex::new(None);

#[derive(Clone, Debug)]
struct ProxyState {
    enabled: bool,
    server: String,
    override_list: String,
}

// ============================================================
// Windows 平台实现
// ============================================================
#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use winreg::enums::*;
    use winreg::RegKey;

    pub fn get_current_state() -> Option<ProxyState> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        let key = match hkcu.open_subkey(path) {
            Ok(k) => k,
            Err(_) => return None,
        };

        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        let override_list: String = key.get_value("ProxyOverride").unwrap_or_default();

        Some(ProxyState {
            enabled: enabled == 1,
            server,
            override_list,
        })
    }

    pub fn set_state(state: &ProxyState) -> anyhow::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        let (key, _) = hkcu.create_subkey(path)?;

        key.set_value("ProxyEnable", &(if state.enabled { 1u32 } else { 0u32 }))?;
        if !state.server.is_empty() {
            key.set_value("ProxyServer", &state.server)?;
        }
        if !state.override_list.is_empty() {
            key.set_value("ProxyOverride", &state.override_list)?;
        }

        // 通知系统代理已更改
        let _ = std::process::Command::new("cmd")
            .args(&["/c", "netsh", "winhttp", "import", "proxy", "source=ie"])
            .output();

        Ok(())
    }

    pub fn enable_proxy(port: u16) -> anyhow::Result<()> {
        let state = ProxyState {
            enabled: true,
            server: format!("127.0.0.1:{}", port),
            override_list: "localhost;127.*;10.*;172.16.*;192.168.*".to_string(),
        };
        set_state(&state)?;
        info!("Windows 系统代理已设置为 127.0.0.1:{}", port);
        Ok(())
    }

    pub fn disable_proxy() -> anyhow::Result<()> {
        let state = ProxyState {
            enabled: false,
            server: String::new(),
            override_list: String::new(),
        };
        set_state(&state)?;
        info!("Windows 系统代理已禁用");
        Ok(())
    }
}

// ============================================================
// macOS 平台实现
// ============================================================
#[cfg(target_os = "macos")]
mod macos_impl {
    use super::*;

    fn get_active_network_service() -> Result<String, Box<dyn std::error::Error>> {
        let output = std::process::Command::new("networksetup")
            .args(&["-listallnetworkservices"])
            .output()?;

        let output = String::from_utf8(output.stdout)?;
        for line in output.lines() {
            if line.contains("Wi-Fi") || line.contains("Ethernet") {
                if !line.contains("disabled") && !line.contains("*") {
                    return Ok(line.trim().to_string());
                }
            }
        }
        Ok("Wi-Fi".to_string())
    }

    pub fn get_current_state() -> Option<ProxyState> {
        // macOS 可以通过 networksetup 查询，但实现较复杂
        // 简化：返回 None 表示未知
        None
    }

    pub fn enable_proxy(port: u16) -> anyhow::Result<()> {
        let wifi_service = get_active_network_service()?;

        std::process::Command::new("networksetup")
            .args(&["-setwebproxy", &wifi_service, "127.0.0.1", &port.to_string()])
            .output()?;

        std::process::Command::new("networksetup")
            .args(&[
                "-setsecurewebproxy",
                &wifi_service,
                "127.0.0.1",
                &port.to_string(),
            ])
            .output()?;

        info!("macOS 系统代理已设置为 127.0.0.1:{}", port);
        Ok(())
    }

    pub fn disable_proxy() -> anyhow::Result<()> {
        let wifi_service = get_active_network_service()?;

        std::process::Command::new("networksetup")
            .args(&["-setwebproxystate", &wifi_service, "off"])
            .output()?;

        std::process::Command::new("networksetup")
            .args(&["-setsecurewebproxystate", &wifi_service, "off"])
            .output()?;

        info!("macOS 系统代理已禁用");
        Ok(())
    }
}

// ============================================================
// Linux 平台实现
// ============================================================
#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;

    pub fn get_current_state() -> Option<ProxyState> {
        // Linux 可以通过 gsettings 查询，但实现较复杂
        // 简化：返回 None 表示未知
        None
    }

    pub fn enable_proxy(port: u16) -> anyhow::Result<()> {
        // GNOME 桌面
        let _ = std::process::Command::new("gsettings")
            .args(&["set", "org.gnome.system.proxy", "mode", "'manual'"])
            .output();

        let _ = std::process::Command::new("gsettings")
            .args(&["set", "org.gnome.system.proxy.http", "host", "'127.0.0.1'"])
            .output();

        let _ = std::process::Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy.http",
                "port",
                &port.to_string(),
            ])
            .output();

        // HTTPS 代理
        let _ = std::process::Command::new("gsettings")
            .args(&["set", "org.gnome.system.proxy.https", "host", "'127.0.0.1'"])
            .output();

        let _ = std::process::Command::new("gsettings")
            .args(&[
                "set",
                "org.gnome.system.proxy.https",
                "port",
                &port.to_string(),
            ])
            .output();

        info!("Linux (GNOME) 系统代理已设置为 127.0.0.1:{}", port);
        Ok(())
    }

    pub fn disable_proxy() -> anyhow::Result<()> {
        let _ = std::process::Command::new("gsettings")
            .args(&["set", "org.gnome.system.proxy", "mode", "'none'"])
            .output();

        info!("Linux (GNOME) 系统代理已禁用");
        Ok(())
    }
}

// ============================================================
// 统一公共接口
// ============================================================

/// 获取当前系统代理状态（平台相关）
fn get_current_state() -> Option<ProxyState> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::get_current_state()
    }
    #[cfg(target_os = "macos")]
    {
        macos_impl::get_current_state()
    }
    #[cfg(target_os = "linux")]
    {
        linux_impl::get_current_state()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// 启用 FastLane 代理（自动设置系统代理）
pub fn enable_proxy(port: u16) -> anyhow::Result<()> {
    // 保存原始状态
    let current_state = get_current_state();
    {
        let mut guard = ORIGINAL_PROXY_STATE.lock().unwrap();
        if guard.is_none() {
            *guard = current_state;
            info!("已保存原始代理配置");
        }
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::enable_proxy(port)?;
    }
    #[cfg(target_os = "macos")]
    {
        macos_impl::enable_proxy(port)?;
    }
    #[cfg(target_os = "linux")]
    {
        linux_impl::enable_proxy(port)?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        info!("当前平台不支持自动设置系统代理，请手动配置");
    }

    Ok(())
}

/// 禁用 FastLane 代理（恢复原始配置）
pub fn disable_proxy() -> anyhow::Result<()> {
    let guard = ORIGINAL_PROXY_STATE.lock().unwrap();

    // 如果有保存的原始状态，尝试恢复
    if let Some(ref original_state) = *guard {
        #[cfg(target_os = "windows")]
        {
            windows_impl::set_state(original_state)?;
            info!("已恢复原始代理配置");
            return Ok(());
        }
        // macOS 和 Linux 目前不支持恢复到原始状态
        // 因为 networksetup 和 gsettings 需要更多信息才能恢复
    }

    // 如果没有保存的原始状态，或者非 Windows 平台，直接关闭代理
    #[cfg(target_os = "windows")]
    {
        windows_impl::disable_proxy()?;
    }
    #[cfg(target_os = "macos")]
    {
        macos_impl::disable_proxy()?;
    }
    #[cfg(target_os = "linux")]
    {
        linux_impl::disable_proxy()?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        info!("当前平台不支持自动取消系统代理设置");
    }

    Ok(())
}