import { test, expect, _electron as electron } from '@playwright/test';
import { getConfig, waitForConfigUpdate, resetConfig, Client } from '../fixtures/test-helpers';

test.describe('Client Name Change', () => {
  test.beforeEach(async () => {
    // Reset config before each test
    await resetConfig();
  });

  test('changing client name persists to config and updates UI', async () => {
    // Launch the Tauri app
    const app = await electron.launch({
      args: ['./src-tauri/target/debug/local-router-ai'],
      env: {
        ...process.env,
        LOCALROUTER_ENV: 'test',
        LOCALROUTER_KEYCHAIN: 'file',
      },
    });

    const window = await app.firstWindow();
    await window.waitForLoadState('domcontentloaded');

    try {
      // 1. Navigate to Clients tab
      await window.click('text=Clients');
      await window.waitForSelector('text=Create Client', { timeout: 10000 });

      // 2. Create a new client
      await window.click('button:has-text("Create Client")');

      // Wait for modal to open
      await window.waitForSelector('input[placeholder="My Application"]', { timeout: 5000 });

      // Fill in the client name
      await window.fill('input[placeholder="My Application"]', 'Original Name');

      // Click Create Client button in the modal
      await window.click('button:has-text("Create Client"):not([disabled])');

      // 3. Wait for credentials modal and close it
      await window.waitForSelector('text=Client Created Successfully', { timeout: 10000 });
      await window.click('button:has-text("Done")');

      // 4. Verify client appears in the list
      await expect(window.locator('text=Original Name')).toBeVisible({ timeout: 5000 });

      // 5. Read config to get client ID
      const configBefore = await getConfig();
      const client = configBefore.clients?.find((c: Client) => c.name === 'Original Name');
      expect(client).toBeTruthy();
      expect(client?.client_id).toBeTruthy();

      // 6. Navigate to client detail page by clicking on the client row
      await window.click('h3:has-text("Original Name")');

      // 7. Wait for detail page to load and switch to Configuration tab
      await window.waitForSelector('text=Configuration', { timeout: 5000 });
      await window.click('text=Configuration');

      // 8. Wait for the settings section to load
      await window.waitForSelector('text=Client Settings', { timeout: 5000 });

      // 9. Clear and update the client name input
      const nameInput = window.locator('input').filter({ hasText: '' }).nth(0);
      // Find the input in the Client Settings section
      const settingsCard = window.locator('text=Client Settings').locator('..');
      const clientNameInput = settingsCard.locator('input').first();

      await clientNameInput.clear();
      await clientNameInput.fill('Updated Name');

      // 10. Click Save Settings
      await window.click('button:has-text("Save Settings")');

      // 11. Wait for save to complete (alert or UI update)
      // Since Tauri doesn't support native alerts well, we check for the UI to update
      await window.waitForTimeout(1000);

      // 12. Verify UI updated - the page title should now show the updated name
      await expect(window.locator('h1:has-text("Updated Name"), h2:has-text("Updated Name"), text=Updated Name').first()).toBeVisible({ timeout: 5000 });

      // 13. Verify config file updated
      const configAfter = await waitForConfigUpdate(
        (config) => {
          const updatedClient = config.clients?.find((c: Client) => c.client_id === client?.client_id);
          return updatedClient?.name === 'Updated Name';
        },
        5000
      );

      const updatedClient = configAfter.clients?.find((c: Client) => c.client_id === client?.client_id);
      expect(updatedClient).toBeTruthy();
      expect(updatedClient?.name).toBe('Updated Name');

      // 14. Navigate back to clients list
      await window.click('button:has-text("Back")');

      // 15. Verify the list shows the updated name
      await expect(window.locator('h3:has-text("Updated Name")')).toBeVisible({ timeout: 5000 });

      // 16. Reload the app and verify persistence
      await window.reload();
      await window.waitForLoadState('domcontentloaded');
      await window.click('text=Clients');
      await expect(window.locator('text=Updated Name')).toBeVisible({ timeout: 10000 });

    } finally {
      await app.close();
    }
  });

  test('client name change reflects after page reload', async () => {
    const app = await electron.launch({
      args: ['./src-tauri/target/debug/local-router-ai'],
      env: {
        ...process.env,
        LOCALROUTER_ENV: 'test',
        LOCALROUTER_KEYCHAIN: 'file',
      },
    });

    const window = await app.firstWindow();
    await window.waitForLoadState('domcontentloaded');

    try {
      // Create a client
      await window.click('text=Clients');
      await window.waitForSelector('button:has-text("Create Client")', { timeout: 10000 });
      await window.click('button:has-text("Create Client")');
      await window.waitForSelector('input[placeholder="My Application"]', { timeout: 5000 });
      await window.fill('input[placeholder="My Application"]', 'Test Client');
      await window.click('button:has-text("Create Client"):not([disabled])');
      await window.waitForSelector('text=Client Created Successfully', { timeout: 10000 });
      await window.click('button:has-text("Done")');

      // Verify it exists
      await expect(window.locator('text=Test Client')).toBeVisible({ timeout: 5000 });

      // Get client ID from config
      const config = await getConfig();
      const client = config.clients?.find((c: Client) => c.name === 'Test Client');
      expect(client?.client_id).toBeTruthy();

      // Go to detail page and change name
      await window.click('h3:has-text("Test Client")');
      await window.click('text=Configuration');
      await window.waitForSelector('text=Client Settings', { timeout: 5000 });

      const settingsCard = window.locator('text=Client Settings').locator('..');
      const clientNameInput = settingsCard.locator('input').first();
      await clientNameInput.clear();
      await clientNameInput.fill('Renamed Client');
      await window.click('button:has-text("Save Settings")');

      // Wait for config to be written
      await waitForConfigUpdate(
        (cfg) => cfg.clients?.some((c: Client) => c.name === 'Renamed Client') ?? false,
        5000
      );

      // Close and relaunch app
      await app.close();

      const app2 = await electron.launch({
        args: ['./src-tauri/target/debug/local-router-ai'],
        env: {
          ...process.env,
          LOCALROUTER_ENV: 'test',
          LOCALROUTER_KEYCHAIN: 'file',
        },
      });

      const window2 = await app2.firstWindow();
      await window2.waitForLoadState('domcontentloaded');

      // Navigate to Clients and verify the renamed client persisted
      await window2.click('text=Clients');
      await expect(window2.locator('text=Renamed Client')).toBeVisible({ timeout: 10000 });

      // The old name should not exist
      await expect(window2.locator('h3:has-text("Test Client")')).not.toBeVisible({ timeout: 2000 });

      await app2.close();
    } catch (error) {
      await app.close();
      throw error;
    }
  });
});
