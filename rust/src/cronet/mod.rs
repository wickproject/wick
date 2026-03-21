pub mod ffi;

use anyhow::{bail, Result};
use std::ffi::CString;
use std::os::raw::c_void;
use std::path::Path;
use std::sync::Mutex;
use tokio::sync::oneshot;

const READ_BUFFER_SIZE: u64 = 32 * 1024; // 32KB per read

/// A Cronet-backed HTTP engine with Chrome-identical TLS/HTTP2/QUIC.
pub struct Engine {
    engine: ffi::Cronet_EnginePtr,
    executor: ffi::Cronet_ExecutorPtr,
}

unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

impl Engine {
    pub fn new(storage_path: &Path, user_agent: &str) -> Result<Self> {
        std::fs::create_dir_all(storage_path)?;

        let ua = CString::new(user_agent)?;
        let sp = CString::new(storage_path.to_string_lossy().as_ref())?;

        unsafe {
            let params = ffi::Cronet_EngineParams_Create();
            ffi::Cronet_EngineParams_enable_http2_set(params, true);
            ffi::Cronet_EngineParams_enable_quic_set(params, true);
            ffi::Cronet_EngineParams_enable_brotli_set(params, true);
            ffi::Cronet_EngineParams_user_agent_set(params, ua.as_ptr());
            ffi::Cronet_EngineParams_storage_path_set(params, sp.as_ptr());
            ffi::Cronet_EngineParams_http_cache_mode_set(params, ffi::HTTP_CACHE_MODE_DISK);
            ffi::Cronet_EngineParams_http_cache_max_size_set(params, 50 * 1024 * 1024);
            ffi::Cronet_EngineParams_enable_check_result_set(params, false);

            let engine = ffi::Cronet_Engine_Create();
            let result = ffi::Cronet_Engine_StartWithParams(engine, params);
            ffi::Cronet_EngineParams_Destroy(params);

            if result != ffi::RESULT_SUCCESS {
                ffi::Cronet_Engine_Destroy(engine);
                bail!("Cronet engine start failed: result={}", result);
            }

            let executor = ffi::Cronet_Executor_CreateWith(executor_execute);

            Ok(Self { engine, executor })
        }
    }

    /// Make an HTTP GET request. Returns (status_code, body).
    pub async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        let (tx, rx) = oneshot::channel();

        // All raw pointer work happens synchronously before the await.
        // Cronet callbacks own cleanup of request/callback/state after Start.
        self.start_request(url, headers, tx)?;

        // Only the receiver crosses the await boundary — it's Send.
        rx.await.map_err(|_| anyhow::anyhow!("request cancelled"))?
    }

    fn start_request(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        tx: oneshot::Sender<Result<Response>>,
    ) -> Result<()> {
        let state = Box::new(RequestState {
            body: Mutex::new(Vec::new()),
            status_code: std::sync::atomic::AtomicI32::new(0),
            sender: Mutex::new(Some(tx)),
            error: Mutex::new(None),
            request: Mutex::new(std::ptr::null_mut()),
            callback: Mutex::new(std::ptr::null_mut()),
        });
        let state_ptr = Box::into_raw(state) as *mut c_void;

        let c_url = CString::new(url)?;

        unsafe {
            let callback = ffi::Cronet_UrlRequestCallback_CreateWith(
                on_redirect_received,
                on_response_started,
                on_read_completed,
                on_succeeded,
                on_failed,
                on_canceled,
            );
            ffi::Cronet_UrlRequestCallback_SetClientContext(callback, state_ptr);

            let params = ffi::Cronet_UrlRequestParams_Create();
            let method = CString::new("GET").unwrap();
            ffi::Cronet_UrlRequestParams_http_method_set(params, method.as_ptr());

            for (name, value) in headers {
                let h = ffi::Cronet_HttpHeader_Create();
                let cn = CString::new(*name)?;
                let cv = CString::new(*value)?;
                ffi::Cronet_HttpHeader_name_set(h, cn.as_ptr());
                ffi::Cronet_HttpHeader_value_set(h, cv.as_ptr());
                ffi::Cronet_UrlRequestParams_request_headers_add(params, h);
                ffi::Cronet_HttpHeader_Destroy(h);
            }

            let request = ffi::Cronet_UrlRequest_Create();
            let result = ffi::Cronet_UrlRequest_InitWithParams(
                request, self.engine, c_url.as_ptr(), params, callback, self.executor,
            );
            ffi::Cronet_UrlRequestParams_Destroy(params);

            if result != ffi::RESULT_SUCCESS {
                let _ = Box::from_raw(state_ptr as *mut RequestState);
                ffi::Cronet_UrlRequest_Destroy(request);
                ffi::Cronet_UrlRequestCallback_Destroy(callback);
                bail!("Cronet request init failed: result={}", result);
            }

            // Store pointers for cleanup in callbacks
            let state_ref = &*(state_ptr as *const RequestState);
            *state_ref.request.lock().unwrap() = request;
            *state_ref.callback.lock().unwrap() = callback;

            let start_result = ffi::Cronet_UrlRequest_Start(request);
            if start_result != ffi::RESULT_SUCCESS {
                let _ = Box::from_raw(state_ptr as *mut RequestState);
                ffi::Cronet_UrlRequest_Destroy(request);
                ffi::Cronet_UrlRequestCallback_Destroy(callback);
                bail!("Cronet request start failed: result={}", start_result);
            }

            Ok(())
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            ffi::Cronet_Engine_Shutdown(self.engine);
            ffi::Cronet_Engine_Destroy(self.engine);
            ffi::Cronet_Executor_Destroy(self.executor);
        }
    }
}

pub struct Response {
    pub status_code: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

// ── Internal state ──────────────────────────────────────────────

struct RequestState {
    body: Mutex<Vec<u8>>,
    status_code: std::sync::atomic::AtomicI32,
    sender: Mutex<Option<oneshot::Sender<Result<Response>>>>,
    error: Mutex<Option<String>>,
    request: Mutex<ffi::Cronet_UrlRequestPtr>,
    callback: Mutex<ffi::Cronet_UrlRequestCallbackPtr>,
}

fn get_state(callback: ffi::Cronet_UrlRequestCallbackPtr) -> &'static RequestState {
    unsafe {
        let ctx = ffi::Cronet_UrlRequestCallback_GetClientContext(callback);
        &*(ctx as *const RequestState)
    }
}

fn complete(state: &RequestState, result: Result<Response>) {
    if let Some(tx) = state.sender.lock().unwrap().take() {
        let _ = tx.send(result);
    }
}

// ── Executor callback ───────────────────────────────────────────

unsafe extern "C" fn executor_execute(
    _executor: ffi::Cronet_ExecutorPtr,
    command: ffi::Cronet_RunnablePtr,
) {
    // Run the command on the current thread (Cronet's network thread).
    ffi::Cronet_Runnable_Run(command);
    ffi::Cronet_Runnable_Destroy(command);
}

// ── Request callbacks ───────────────────────────────────────────

unsafe extern "C" fn on_redirect_received(
    _callback: ffi::Cronet_UrlRequestCallbackPtr,
    request: ffi::Cronet_UrlRequestPtr,
    _info: ffi::Cronet_UrlResponseInfoPtr,
    _new_location: ffi::Cronet_String,
) {
    // Follow all redirects automatically
    ffi::Cronet_UrlRequest_FollowRedirect(request);
}

unsafe extern "C" fn on_response_started(
    callback: ffi::Cronet_UrlRequestCallbackPtr,
    request: ffi::Cronet_UrlRequestPtr,
    info: ffi::Cronet_UrlResponseInfoPtr,
) {
    let state = get_state(callback);
    let code = ffi::Cronet_UrlResponseInfo_http_status_code_get(info);
    state.status_code.store(code, std::sync::atomic::Ordering::SeqCst);

    // Start reading the response body
    let buffer = ffi::Cronet_Buffer_Create();
    ffi::Cronet_Buffer_InitWithAlloc(buffer, READ_BUFFER_SIZE);
    ffi::Cronet_UrlRequest_Read(request, buffer);
}

unsafe extern "C" fn on_read_completed(
    callback: ffi::Cronet_UrlRequestCallbackPtr,
    request: ffi::Cronet_UrlRequestPtr,
    _info: ffi::Cronet_UrlResponseInfoPtr,
    buffer: ffi::Cronet_BufferPtr,
    bytes_read: u64,
) {
    let state = get_state(callback);

    // Append read data to body
    let data_ptr = ffi::Cronet_Buffer_GetData(buffer) as *const u8;
    let slice = std::slice::from_raw_parts(data_ptr, bytes_read as usize);
    state.body.lock().unwrap().extend_from_slice(slice);

    // Continue reading
    ffi::Cronet_Buffer_InitWithAlloc(buffer, READ_BUFFER_SIZE);
    ffi::Cronet_UrlRequest_Read(request, buffer);
}

/// Clean up Cronet objects and free the state.
unsafe fn finish_request(callback: ffi::Cronet_UrlRequestCallbackPtr) -> Box<RequestState> {
    let ctx = ffi::Cronet_UrlRequestCallback_GetClientContext(callback);
    let boxed = Box::from_raw(ctx as *mut RequestState);
    let req = *boxed.request.lock().unwrap();
    let cb = *boxed.callback.lock().unwrap();
    // Note: Cronet buffers are reused across on_read_completed callbacks
    // and destroyed internally by Cronet when the request completes.
    if !req.is_null() {
        ffi::Cronet_UrlRequest_Destroy(req);
    }
    if !cb.is_null() {
        ffi::Cronet_UrlRequestCallback_Destroy(cb);
    }
    boxed
}

unsafe extern "C" fn on_succeeded(
    callback: ffi::Cronet_UrlRequestCallbackPtr,
    _request: ffi::Cronet_UrlRequestPtr,
    _info: ffi::Cronet_UrlResponseInfoPtr,
) {
    let state = get_state(callback);
    let code = state.status_code.load(std::sync::atomic::Ordering::SeqCst);
    let body = state.body.lock().unwrap().clone();

    let boxed = finish_request(callback);
    complete(
        &boxed,
        Ok(Response {
            status_code: code as u16,
            body,
        }),
    );
}

unsafe extern "C" fn on_failed(
    callback: ffi::Cronet_UrlRequestCallbackPtr,
    _request: ffi::Cronet_UrlRequestPtr,
    _info: ffi::Cronet_UrlResponseInfoPtr,
    _error: ffi::Cronet_ErrorPtr,
) {
    let boxed = finish_request(callback);
    complete(&boxed, Err(anyhow::anyhow!("Cronet request failed")));
}

unsafe extern "C" fn on_canceled(
    callback: ffi::Cronet_UrlRequestCallbackPtr,
    _request: ffi::Cronet_UrlRequestPtr,
    _info: ffi::Cronet_UrlResponseInfoPtr,
) {
    let boxed = finish_request(callback);
    complete(&boxed, Err(anyhow::anyhow!("Cronet request cancelled")));
}
