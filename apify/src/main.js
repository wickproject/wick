import { Actor, log } from 'apify';
import { spawn } from 'child_process';

const WICK_PORT = 18090;
const WICK_BASE = `http://127.0.0.1:${WICK_PORT}`;

await Actor.init();

const input = (await Actor.getInput()) ?? {};
const {
    urls,
    mode = 'fetch',
    format = 'markdown',
    maxPages = 10,
    maxDepth = 2,
    mapLimit = 100,
    wickTunnelUrl,
    wickApiKey,
} = input;

if (!Array.isArray(urls) || urls.length === 0) {
    log.error('Input must include a non-empty "urls" array.');
    await Actor.exit({ exitCode: 1 });
}

const dataset = await Actor.openDataset();
const useTunnel = !!wickTunnelUrl;
const baseUrl = useTunnel ? wickTunnelUrl.replace(/\/$/, '') : WICK_BASE;
const headers = wickApiKey ? { Authorization: `Bearer ${wickApiKey}` } : {};

// Start the bundled Wick API server if not using a tunnel
let wickProcess;
if (!useTunnel) {
    log.info('Starting Wick API server...');
    wickProcess = spawn('/usr/local/bin/wick', ['serve', '--api', '--port', String(WICK_PORT)], {
        env: { ...process.env, LD_LIBRARY_PATH: '/usr/local/lib' },
        stdio: ['ignore', 'pipe', 'pipe'],
    });

    wickProcess.on('error', (err) => {
        log.error(`Failed to start Wick: ${err.message}`);
        Actor.exit({ exitCode: 1 });
    });

    wickProcess.stdout.on('data', (chunk) => {
        const msg = chunk.toString().trimEnd();
        if (msg) log.info(`[wick] ${msg}`);
    });
    wickProcess.stderr.on('data', (chunk) => {
        const msg = chunk.toString().trimEnd();
        if (msg) log.warning(`[wick] ${msg}`);
    });

    // Wait for server to be ready
    let ready = false;
    for (let i = 0; i < 30; i++) {
        try {
            const resp = await fetch(`${WICK_BASE}/health`);
            if (resp.ok) { ready = true; break; }
        } catch { /* not ready yet */ }
        await new Promise(r => setTimeout(r, 500));
    }

    if (!ready) {
        log.error('Wick API server failed to start within 15s');
        await Actor.exit({ exitCode: 1 });
    }
    log.info('Wick API server ready');
} else {
    log.info(`Using Wick tunnel at ${wickTunnelUrl}`);
}

async function wickFetch(url) {
    const params = new URLSearchParams({ url, format });
    const resp = await fetch(`${baseUrl}/v1/fetch?${params}`, { headers });
    if (!resp.ok) throw new Error(`Wick returned ${resp.status}: ${await resp.text()}`);
    return resp.json();
}

async function wickCrawl(url) {
    const params = new URLSearchParams({
        url, format, max_pages: String(maxPages), max_depth: String(maxDepth),
    });
    const resp = await fetch(`${baseUrl}/v1/crawl?${params}`, { headers });
    if (!resp.ok) throw new Error(`Wick returned ${resp.status}: ${await resp.text()}`);
    return resp.json();
}

async function wickMap(url) {
    const params = new URLSearchParams({ url, limit: String(mapLimit) });
    const resp = await fetch(`${baseUrl}/v1/map?${params}`, { headers });
    if (!resp.ok) throw new Error(`Wick returned ${resp.status}: ${await resp.text()}`);
    return resp.json();
}

const engine = useTunnel ? 'wick-tunnel' : 'wick-local';

for (const url of urls) {
    try {
        log.info(`${mode}: ${url}`);

        if (mode === 'crawl') {
            const result = await wickCrawl(url);
            for (const page of result.pages || []) {
                await dataset.pushData({
                    url: page.url,
                    title: page.title || null,
                    content: page.content,
                    format,
                    fetchedAt: new Date().toISOString(),
                    engine,
                });
            }
            log.info(`Crawled ${result.pages?.length || 0} pages from ${url}`);
        } else if (mode === 'map') {
            const result = await wickMap(url);
            await dataset.pushData({
                url,
                urls: result.urls,
                format: 'urls',
                timingMs: result.timing_ms,
                fetchedAt: new Date().toISOString(),
                engine,
            });
            log.info(`Mapped ${result.count} URLs from ${url}`);
        } else {
            const result = await wickFetch(url);
            await dataset.pushData({
                url,
                title: result.title || null,
                content: result.content,
                statusCode: result.status,
                timingMs: result.timing_ms,
                format,
                fetchedAt: new Date().toISOString(),
                engine,
            });
        }
    } catch (err) {
        log.error(`Failed: ${url}: ${err.message}`);
        await dataset.pushData({
            url,
            error: err.message,
            fetchedAt: new Date().toISOString(),
            engine,
        });
    }
}

// Clean up
if (wickProcess) {
    wickProcess.kill('SIGTERM');
}

await Actor.exit();
