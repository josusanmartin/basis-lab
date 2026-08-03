const { test, expect } = require('@playwright/test');

const deployedBaseUrl = process.env.DEPLOYED_BASE_URL;

test.describe('deployed static preview', () => {
  test.skip(!deployedBaseUrl, 'Set DEPLOYED_BASE_URL to run production smoke tests');

  test('loads assets, interactive candles, docs, and the OpenAPI contract', async ({ page, request }) => {
    const pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));

    const deployedPage = new URL(deployedBaseUrl);
    deployedPage.searchParams.set('qa', Date.now().toString());
    deployedPage.searchParams.set('static_demo', '1');
    await page.goto(deployedPage.href);
    await expect(page).toHaveTitle(/Basis Lab/);
    await expect(page.locator('#health-label')).toHaveText('Static demo');
    await expect(page.locator('#chart-pair')).toHaveText('WLFIUSDT / WLFI_USDT');
    await expect(page.locator('#chart-subtitle')).toContainText('illustrative static data');
    await expect(page.locator('#observations-body tr')).toHaveCount(12);

    await page.locator('#swap').click();
    await expect(page.locator('#chart-pair')).toHaveText('WLFI_USDT / WLFIUSDT');

    const spec = await request.get(new URL('openapi.json', deployedBaseUrl).href);
    expect(spec.ok()).toBeTruthy();
    const contract = await spec.json();
    expect(contract.paths['/api/v1/compare']).toBeTruthy();
    expect(contract.paths['/api/v1/tickers']).toBeTruthy();

    await page.goto(new URL('docs.html', deployedBaseUrl).href);
    await expect(page.getByRole('heading', { name: 'Basis Lab API' })).toBeVisible();
    await expect(page.locator('#hosting-note')).toBeVisible();
    expect(pageErrors).toEqual([]);
  });
});
