const { test, expect } = require('@playwright/test');

const liveBaseUrl = process.env.LIVE_DEPLOYED_BASE_URL;
const liveApiBaseUrl = process.env.LIVE_DEPLOYED_API_BASE_URL || liveBaseUrl;

test.describe('deployed live Rust service', () => {
  test.skip(!liveBaseUrl, 'Set LIVE_DEPLOYED_BASE_URL to run live production E2E');

  test('serves real venue data through the UI and agent API', async ({ page, request }, testInfo) => {
    test.setTimeout(60_000);
    const pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));

    await page.goto(liveBaseUrl);
    await expect(page.locator('#health-label')).toHaveText('Live', { timeout: 30_000 });
    await expect(page.locator('#chart-pair')).toHaveText('WLFIUSDT / WLFI_USDT');
    await expect(page.locator('#chart-subtitle')).toContainText('aligned 1h candles · bps');
    await expect(page.locator('#metric-latest')).not.toHaveText('—');
    await expect(page.locator('#observations-body tr')).toHaveCount(12);

    const canvas = page.locator('#chart');
    const box = await canvas.boundingBox();
    expect(box.width).toBeGreaterThan(testInfo.project.name.includes('mobile') ? 280 : 700);
    await canvas.hover({ position: { x: Math.round(box.width * 0.7), y: Math.round(box.height * 0.4) } });
    await expect(page.locator('#chart-tooltip')).toContainText('UTC');

    const health = await request.get(new URL('/api/v1/health', liveApiBaseUrl).href);
    expect(health.ok()).toBeTruthy();
    expect((await health.json()).service).toBe('basis-lab');

    const venues = await request.get(new URL('/api/v1/venues', liveApiBaseUrl).href);
    expect(venues.ok()).toBeTruthy();
    expect((await venues.json()).data).toHaveLength(12);

    const tickers = await request.get(new URL('/api/v1/tickers?query=WLFI%2FUSDT&limit=100', liveApiBaseUrl).href);
    expect(tickers.ok()).toBeTruthy();
    expect(tickers.headers()['cache-control']).toContain('stale-while-revalidate');
    const tickerBody = await tickers.json();
    expect(tickerBody.cache_ttl_seconds).toBe(300);
    expect(tickerBody.data.some(ticker => ticker.normalized_symbol === 'WLFI/USDT')).toBeTruthy();

    const hip3 = await request.get(new URL('/api/v1/markets?venue=hyperliquid_perp&query=SPCX&limit=100', liveApiBaseUrl).href);
    expect(hip3.ok()).toBeTruthy();
    expect((await hip3.json()).data.some(market => market.symbol === 'xyz:SPCX')).toBeTruthy();

    const suggestions = await request.get(new URL('/api/v1/tickers/suggest?source_venue=ondo_perp&source_symbol=SPCX-USD.P&target_venue=hyperliquid_perp', liveApiBaseUrl).href);
    expect(suggestions.ok()).toBeTruthy();
    const suggestionBody = await suggestions.json();
    expect(suggestionBody.data[0].symbol).toBe('xyz:SPCX');
    expect(suggestionBody.data[0].confidence).toBe(1);

    const spec = await request.get(new URL('/openapi.json', liveApiBaseUrl).href);
    const contract = await spec.json();
    expect(contract.components.schemas.ComparisonResponse.required).toContain('candles');
    expect(contract.components.schemas.Venue.enum).toHaveLength(12);
    expect(pageErrors).toEqual([]);
  });
});
