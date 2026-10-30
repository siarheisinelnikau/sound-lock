//! Windows' per-app policy interface is internal. Its two known IIDs and ABI
//! slots are documented by EarTrumpet's interoperability sources; no helper
//! program, registry-wide reset or system-default change is used here.
use super::*;
use ::windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE},
        Media::Audio::{
            AudioSessionStateExpired, DEVICE_STATE_ACTIVE, ERole, IAudioSessionControl2,
            IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree},
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
            WinRT::{RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize, RoUninitialize},
        },
    },
    core::{GUID, HRESULT, HSTRING, IInspectable_Vtbl, IUnknown, Interface, PWSTR},
};
use std::{collections::HashSet, ffi::c_void};

#[repr(C)]
struct PolicyVtable {
    base: IInspectable_Vtbl,
    reserved: [usize; 19],
    set: unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> HRESULT,
    get: unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut *mut c_void) -> HRESULT,
}
#[repr(transparent)]
#[derive(Clone)]
struct Current(IUnknown);
// SAFETY: both wrappers retain an owned COM pointer with the exact ABI above.
unsafe impl Interface for Current {
    type Vtable = PolicyVtable;
    const IID: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);
}
#[repr(transparent)]
#[derive(Clone)]
struct Downlevel(IUnknown);
unsafe impl Interface for Downlevel {
    type Vtable = PolicyVtable;
    const IID: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);
}
enum Policy {
    Current(Current),
    Downlevel(Downlevel),
}
impl Policy {
    fn open() -> Result<Self> {
        let class = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
        // SAFETY: the worker initialized WinRT and owns these interfaces until
        // before RoUninitialize. The old IID is tried only if the modern fails.
        unsafe {
            if let Ok(factory) = RoGetActivationFactory::<Current>(&class) {
                return Ok(Self::Current(factory));
            }
            Ok(Self::Downlevel(
                RoGetActivationFactory::<Downlevel>(&class).context(
                    "This Windows version does not expose compatible per-app audio routing",
                )?,
            ))
        }
    }
    fn raw(&self) -> (*mut c_void, &PolicyVtable) {
        match self {
            Self::Current(v) => (v.as_raw(), v.vtable()),
            Self::Downlevel(v) => (v.as_raw(), v.vtable()),
        }
    }
    fn get(&self, pid: u32, role: usize) -> Result<Option<String>> {
        let (raw, vtable) = self.raw();
        let mut result = std::ptr::null_mut();
        // SAFETY: Get returns an owned HSTRING handle (not a UTF-16 pointer).
        let status = unsafe { (vtable.get)(raw, pid, 0, role as i32, &mut result) };
        if status.0 == 0x80070490_u32 as i32 {
            return Ok(None);
        } // No persisted preference.
        status
            .ok()
            .context("Could not read the browser's previous output")?;
        let value: HSTRING = unsafe { std::mem::transmute(result) };
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(endpoint_id(&value.to_string_lossy())))
        }
    }
    fn set(&self, pid: u32, role: usize, endpoint: Option<&str>) -> Result<()> {
        let (raw, vtable) = self.raw();
        let value = endpoint
            .map(|id| HSTRING::from(format!("{PREFIX}{}{SUFFIX}", endpoint_id(id))))
            .unwrap_or_default();
        // SAFETY: HSTRING is repr(transparent) over its ABI handle and remains
        // alive throughout the call. Empty HSTRING restores Follow default.
        let handle = unsafe { std::mem::transmute_copy::<HSTRING, *mut c_void>(&value) };
        unsafe { (vtable.set)(raw, pid, 0, role as i32, handle).ok() }
            .context("Windows could not change the browser output")?;
        Ok(())
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub(super) struct Platform {
    policy: Policy,
    enumerator: IMMDeviceEnumerator,
    _apartment: Apartment,
}
impl Platform {
    pub fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let apartment = Apartment;
        let policy = Policy::open()?;
        let enumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        Ok(Self {
            policy,
            enumerator,
            _apartment: apartment,
        })
    }
    fn audio_pids(&self) -> Result<HashSet<u32>> {
        let mut pids = HashSet::new();
        unsafe {
            let devices = self
                .enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let Ok(device) = devices.Item(i) else {
                    continue;
                };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                    continue;
                };
                let Ok(sessions) = manager.GetSessionEnumerator() else {
                    continue;
                };
                for index in 0..sessions.GetCount()? {
                    let Ok(session) = sessions.GetSession(index) else {
                        continue;
                    };
                    if session
                        .GetState()
                        .is_ok_and(|s| s == AudioSessionStateExpired)
                    {
                        continue;
                    }
                    if let Ok(session) = session.cast::<IAudioSessionControl2>()
                        && let Ok(pid) = session.GetProcessId()
                        && pid != 0
                    {
                        pids.insert(pid);
                    }
                }
            }
        }
        Ok(pids)
    }
    fn info(pid: u32, browser: Browser, has_audio: bool) -> Result<(Process, Handle)> {
        // A live process handle prevents PID reuse throughout the policy call.
        let handle = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)? });
        let mut path = vec![0_u16; 32_768];
        let mut length = path.len() as u32;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe {
            QueryFullProcessImageNameW(
                handle.0,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut length,
            )?;
            GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user)?;
        }
        let path = String::from_utf16(&path[..length as usize])?;
        let created = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        Ok((
            Process {
                pid,
                created,
                path,
                browser,
                has_audio,
            },
            handle,
        ))
    }
    fn lease(process: &Process) -> Result<Handle> {
        let (current, handle) = Self::info(process.pid, process.browser, process.has_audio)?;
        ensure!(
            current.created == process.created && current.path.eq_ignore_ascii_case(&process.path),
            "Browser process changed before routing; retry Start"
        );
        Ok(handle)
    }
    fn wide(id: &str) -> Vec<u16> {
        endpoint_id(id).encode_utf16().chain(Some(0)).collect()
    }
}
impl Backend for Platform {
    fn processes(&mut self) -> Result<Vec<Process>> {
        let audio = self.audio_pids()?;
        let snapshot = Handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? });
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut result = Vec::new();
        let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        while next.is_ok() {
            let len = entry
                .szExeFile
                .iter()
                .position(|x| *x == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if let Some(browser) = Browser::ALL
                .into_iter()
                .find(|b| exe.eq_ignore_ascii_case(b.exe()))
                && let Ok((process, _handle)) = Self::info(
                    entry.th32ProcessID,
                    browser,
                    audio.contains(&entry.th32ProcessID),
                )
            {
                result.push(process);
            }
            next = unsafe { Process32NextW(snapshot.0, &mut entry) };
        }
        result.sort_by_key(|p| !p.has_audio);
        Ok(result)
    }
    fn get(&mut self, process: &Process, role: usize) -> Result<Option<String>> {
        let _lease = Self::lease(process)?;
        self.policy.get(process.pid, role)
    }
    fn set(&mut self, process: &Process, role: usize, endpoint: Option<&str>) -> Result<()> {
        let _lease = Self::lease(process)?;
        self.policy.set(process.pid, role, endpoint)
    }
    fn available(&mut self, endpoint: &str) -> Result<bool> {
        let id = Self::wide(endpoint);
        let device = unsafe {
            self.enumerator
                .GetDevice(::windows::core::PCWSTR(id.as_ptr()))
        };
        Ok(device
            .is_ok_and(|d| unsafe { d.GetState().is_ok_and(|state| state == DEVICE_STATE_ACTIVE) }))
    }
    fn default_endpoint(&mut self, role: usize) -> Result<Option<String>> {
        unsafe {
            let device = self
                .enumerator
                .GetDefaultAudioEndpoint(eRender, ERole(role as i32))?;
            let id = device.GetId()?;
            let value = id.to_string();
            CoTaskMemFree(Some(id.0.cast()));
            Ok(Some(endpoint_id(&value?)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_abi_slots_match_the_windows_interface() {
        assert_eq!(
            std::mem::offset_of!(PolicyVtable, set),
            25 * size_of::<usize>()
        );
        assert_eq!(
            std::mem::offset_of!(PolicyVtable, get),
            26 * size_of::<usize>()
        );
    }
    #[test]
    #[ignore = "Temporarily changes and restores only this test process's audio preferences; no browser audio touched"]
    fn real_policy_roundtrip_restores_own_process_preferences() {
        std::thread::spawn(|| -> Result<()> {
            let mut platform = Platform::new()?;
            // Register an audio session for this otherwise silent test process.
            // The stream is never started and cannot emit sound.
            use cpal::traits::{DeviceTrait, HostTrait};
            let device = cpal::default_host()
                .default_output_device()
                .context("No audio output")?;
            let config = device.default_output_config()?;
            let _stream = device.build_output_stream_raw(
                &config.config(),
                config.sample_format(),
                |_, _| {},
                |_| {},
                None,
            )?;
            let (process, _handle) = Platform::info(std::process::id(), Browser::Chrome, false)?;
            let previous: Vec<_> = (0..3)
                .map(|r| {
                    platform
                        .get(&process, r)
                        .with_context(|| format!("Read original role {r}"))
                })
                .collect::<Result<_>>()?;
            struct RestoreOwn<'a> {
                platform: &'a mut Platform,
                process: &'a Process,
                previous: Vec<Option<String>>,
            }
            impl Drop for RestoreOwn<'_> {
                fn drop(&mut self) {
                    for role in 0..3 {
                        let _ =
                            self.platform
                                .set(self.process, role, self.previous[role].as_deref());
                    }
                }
            }
            let restore = RestoreOwn {
                platform: &mut platform,
                process: &process,
                previous: previous.clone(),
            };
            let target = restore
                .platform
                .default_endpoint(1)?
                .context("No audio output for the policy smoke test")?;
            for role in 0..3 {
                restore.platform.set(&process, role, Some(&target))?;
                ensure!(
                    restore.platform.get(&process, role)?.as_deref() == Some(target.as_str()),
                    "Routing readback differs"
                );
            }
            drop(restore);
            for (role, original) in previous.iter().enumerate() {
                ensure!(
                    platform.get(&process, role)? == *original,
                    "Original preference was not restored"
                );
            }
            Ok(())
        })
        .join()
        .unwrap()
        .unwrap();
    }
}
