const { test, expect } = require('@playwright/test');

async function mockComparisonApi(page) {
  await page.route('**/api/v1/markets?*', route => {
    const requestUrl = new URL(route.request().url());
    const query = requestUrl.searchParams.get('query') || '';
    const venue = requestUrl.searchParams.get('venue');
    const needle = query.replace(/[^a-z0-9]/gi, '').toUpperCase();
    const defaultMarkets = [
      { symbol: 'WLFIUSDT', normalized_symbol: 'WLFI/USDT', base: 'WLFI', quote: 'USDT', active: true },
      { symbol: 'BTCUSDT', normalized_symbol: 'BTC/USDT', base: 'BTC', quote: 'USDT', active: true },
      { symbol: 'ETHUSDT', normalized_symbol: 'ETH/USDT', base: 'ETH', quote: 'USDT', active: true }
    ];
    const venueMarkets = {
      ondo_perp: [
        { symbol: 'SPCX-USD.P', normalized_symbol: 'SPCX/USD', base: 'SPCX', quote: 'USD', active: true },
        { symbol: 'BTC-USD.P', normalized_symbol: 'BTC/USD', base: 'BTC', quote: 'USD', active: true }
      ],
      hyperliquid_perp: [
        { symbol: 'SPX', normalized_symbol: 'SPX/USD', base: 'SPX', quote: 'USD', active: true },
        { symbol: 'xyz:SPCX', normalized_symbol: 'SPCX/USD', base: 'SPCX', quote: 'USD', active: true }
      ]
    };
    const markets = (venueMarkets[venue] || defaultMarkets).filter(market => `${market.symbol}${market.normalized_symbol}${market.base}${market.quote}`.replace(/[^a-z0-9]/gi, '').toUpperCase().includes(needle));
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: markets })
    });
  });
  await page.route('**/api/v1/tickers/suggest?*', route => {
    const requestUrl = new URL(route.request().url());
    const source = requestUrl.searchParams.get('source_symbol') || '';
    const target = requestUrl.searchParams.get('target_venue');
    const spcx = source.toUpperCase().includes('SPCX') && target === 'hyperliquid_perp';
    const ticker = spcx
      ? { symbol: 'xyz:SPCX', normalized_symbol: 'SPCX/USD', base: 'SPCX', quote: 'USD', venue: 'hyperliquid_perp', venue_label: 'Hyperliquid', market_type: 'perpetual' }
      : { symbol: 'BTC_USDT', normalized_symbol: 'BTC/USDT', base: 'BTC', quote: 'USDT', venue: target, venue_label: 'Target venue', market_type: 'perpetual' };
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: [{ ...ticker, confidence: 1, match_reason: 'same normalized base', requires_multiplier_check: false }] })
    });
  });
  await page.route('**/api/v1/compare?*', async route => {
    const requestUrl = new URL(route.request().url());
    if (requestUrl.searchParams.get('scale') !== '10000') {
      return route.fulfill({ status: 400, contentType: 'application/json', body: JSON.stringify({ error: { message: 'basis scale must be 10000' } }) });
    }
    const interval = requestUrl.searchParams.get('interval') || '1h';
    const intervalMs = { '1m': 60e3, '3m': 180e3, '5m': 300e3, '15m': 900e3, '30m': 1800e3, '1h': 3600e3, '2h': 7200e3, '4h': 14400e3, '1d': 86400e3 }[interval];
    const count = 180;
    const end = Math.floor(Date.now() / intervalMs) * intervalMs;
    const candles = Array.from({ length: count }, (_, index) => {
      const time = end - (count - 1 - index) * intervalMs;
      const center = 23 + Math.sin(index / 9) * 16 + index * 0.035;
      const open = center + Math.sin(index * 0.7) * 2.4;
      const close = center + Math.cos(index * 0.55) * 2.7;
      return {
        time,
        open,
        high: Math.max(open, close) + 4.2,
        low: Math.min(open, close) - 4.2,
        close,
        left_close: 0.164 + index * 0.00001,
        right_close: 0.1636 + index * 0.00001
      };
    });
    const closes = candles.map(candle => candle.close);
    const mean = closes.reduce((sum, value) => sum + value, 0) / closes.length;
    const variance = closes.reduce((sum, value) => sum + (value - mean) ** 2, 0) / closes.length;
    const standardDeviation = Math.sqrt(variance);
    const latest = closes.at(-1);
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        interval,
        unit: 'basis points',
        scale: 10000,
        matched_candles: count,
        dropped_left: 0,
        dropped_right: 0,
        candles,
        stats: {
          latest,
          mean,
          standard_deviation: standardDeviation,
          minimum: Math.min(...closes),
          maximum: Math.max(...closes),
          z_score: (latest - mean) / standardDeviation
        }
      })
    });
  });
}

test.describe('Basis Lab browser workflow', () => {
  test('loads the WLFI basis, renders candles, and supports trading controls', async ({ page }, testInfo) => {
    const pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));
    await mockComparisonApi(page);

    await page.goto('/');
    await expect(page).toHaveTitle(/Basis Lab/);
    await expect(page.locator('.intro')).toHaveCount(0);
    await expect(page.locator('#health-label')).toHaveText('Live');
    await expect(page.locator('#chart-pair')).toHaveText('WLFIUSDT / WLFI_USDT');
    await expect(page.locator('#metric-latest')).not.toHaveText('—');
    await expect(page.locator('#upper-entry')).not.toHaveText('—');
    await expect(page.locator('#observations-body tr')).toHaveCount(12);

    const canvas = page.locator('#chart');
    const box = await canvas.boundingBox();
    expect(box.width).toBeGreaterThan(testInfo.project.name.includes('mobile') ? 280 : 700);
    expect(box.height).toBeGreaterThan(300);
    await canvas.hover({ position: { x: Math.round(box.width * 0.7), y: Math.round(box.height * 0.4) } });
    await expect(page.locator('#chart-tooltip')).toBeVisible();
    await expect(page.locator('#chart-tooltip')).toContainText('UTC');

    const previousBand = await page.locator('#upper-entry').textContent();
    await page.locator('#entry-z').fill('3');
    await expect(page.locator('#entry-z-label')).toHaveText('3.0σ');
    await expect(page.locator('#upper-entry')).not.toHaveText(previousBand);

    await page.locator('#swap').click();
    await expect(page.locator('#left-venue')).toHaveValue('mexc_perp');
    await expect(page.locator('#right-venue')).toHaveValue('bybit_perp');
    await expect(page.locator('#chart-pair')).toHaveText('WLFI_USDT / WLFIUSDT');
    await expect(page.locator('#metric-latest')).not.toHaveText('—');

    await page.getByRole('button', { name: '15M' }).click();
    await expect(page.locator('#chart-subtitle')).toContainText('15m candles');
    expect(pageErrors).toEqual([]);
  });

  test('recovers from transient platform routing misses', async ({ page }) => {
    let venueAttempts = 0;
    await mockComparisonApi(page);
    await page.route('**/api/v1/venues', async route => {
      venueAttempts += 1;
      if (venueAttempts < 3) return route.fulfill({ status: 404, body: 'Not Found' });
      return route.continue();
    });

    await page.goto('/');
    await expect(page.locator('#health-label')).toHaveText('Live');
    await expect(page.locator('#chart-pair')).toHaveText('WLFIUSDT / WLFI_USDT');
    expect(venueAttempts).toBe(3);
  });

  test('searches normalized tickers and selects the native venue symbol', async ({ page }) => {
    await mockComparisonApi(page);
    await page.goto('/');

    const input = page.locator('#left-market');
    await input.focus();
    await expect(page.locator('#left-market-options')).toBeVisible();
    await expect(page.locator('#left-market-options .market-option')).toHaveCount(3);

    await input.fill('btc/usdt');
    await expect(page.locator('#left-market-options .market-option')).toHaveCount(1);
    await expect(page.locator('#left-market-options .market-option strong')).toHaveText('BTC/USDT');
    await expect(page.locator('#left-market-options .market-option span')).toHaveText('BTCUSDT');
    await input.press('ArrowDown');
    await input.press('Enter');
    await expect(input).toHaveValue('BTCUSDT');
    await expect(input).toHaveAttribute('aria-expanded', 'false');
  });

  test('suggests the exact Hyperliquid HIP-3 equivalent on the other venue', async ({ page }) => {
    await mockComparisonApi(page);
    await page.goto('/?left_venue=ondo_perp&left_market=SPCX-USD.P&right_venue=hyperliquid_perp&right_market=SPX');

    const left = page.locator('#left-market');
    await left.fill('SPCX');
    await expect(page.locator('#left-market-options .market-option')).toHaveCount(1);
    await left.press('ArrowDown');
    await left.press('Enter');

    const suggestion = page.locator('#right-market-options .market-option');
    await expect(suggestion).toHaveCount(1);
    await expect(suggestion.locator('strong')).toHaveText('SPCX/USD');
    await expect(suggestion.locator('span')).toContainText('xyz:SPCX · same normalized base');
    await suggestion.click();
    await expect(page.locator('#right-market')).toHaveValue('xyz:SPCX');

    await page.locator('#run').click();
    await expect(page.locator('#formula')).toContainText('× 10,000');
    await expect(page.locator('#metric-latest')).not.toHaveText('—');
  });

  test('exposes agent-facing API metadata and rejects unsafe inputs', async ({ request, page }) => {
    const health = await request.get('/api/v1/health');
    expect(health.ok()).toBeTruthy();
    expect((await health.json()).status).toBe('ok');

    const venues = await request.get('/api/v1/venues');
    const venueBody = await venues.json();
    expect(venueBody.data).toHaveLength(12);
    expect(venueBody.data.map(item => item.id)).toContain('ondo_perp');

    const bad = await request.get('/api/v1/candles?venue=bybit_perp&market=../secret?x=1&interval=1h&from=1&to=2');
    expect(bad.status()).toBe(400);
    expect((await bad.json()).error.code).toBe('bad_request');

    const spec = await request.get('/openapi.json');
    expect(spec.ok()).toBeTruthy();
    expect((await spec.json()).paths['/api/v1/compare']).toBeTruthy();

    await page.goto('/docs');
    await expect(page.getByRole('heading', { name: 'Basis Lab API' })).toBeVisible();
    await expect(page.locator('main')).toContainText('Comparison semantics');
  });

  test('keeps the public static preview interactive without an API host', async ({ page }) => {
    await page.goto('/?static_demo=1');
    await expect(page.locator('#health-label')).toHaveText('Static demo');
    await expect(page.locator('#chart-pair')).toHaveText('WLFIUSDT / WLFI_USDT');
    await expect(page.locator('#chart-subtitle')).toContainText('illustrative static data');
    await expect(page.locator('#observations-body tr')).toHaveCount(12);
  });
});
