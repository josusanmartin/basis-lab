const { test, expect } = require('@playwright/test');

test.describe('Basis Lab live workflow', () => {
  test('loads the WLFI basis, renders candles, and supports trading controls', async ({ page }, testInfo) => {
    const pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));

    await page.goto('/');
    await expect(page).toHaveTitle(/Basis Lab/);
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
});

