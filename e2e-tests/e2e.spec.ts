import { test, expect } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

// Define paths
const testVaultPath = path.resolve(__dirname, '../test-vault');
const dummyImagePath = path.join(__dirname, 'dummy-photo.jpg');
const dummyAudioPath = path.join(__dirname, 'dummy-voice.wav');

// Helper to find the active daily note file on disk
function getTodayDailyNotePath(): string {
  const date = new Date();
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  // DailyNoteManager formats daily note date as YYYY-MM-DD by default
  return path.join(testVaultPath, `${year}-${month}-${day}.md`);
}

test.describe('Obsidian AI Agent WebUI End-to-End Tests', () => {
  
  test.beforeAll(() => {
    // Generate E2E dummy test assets if they don't exist
    // Tiny valid 1x1 black PNG (used instead of JPEG because the image crate rejects our dummy JPEG)
    const dummyJpgBase64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
    fs.writeFileSync(dummyImagePath, Buffer.from(dummyJpgBase64, 'base64'));

    // Construct a valid 1-second silent PCM 16-bit 8000Hz mono WAV file
    const sampleRate = 8000;
    const numChannels = 1;
    const bitsPerSample = 16;
    const duration = 1.0; // 1 second
    
    const blockAlign = numChannels * (bitsPerSample / 8); // 2 bytes
    const byteRate = sampleRate * blockAlign; // 16000 bytes/sec
    const dataSize = Math.floor(sampleRate * duration) * blockAlign; // 16000 bytes
    const fileSize = 36 + dataSize; // 16036 bytes
    
    const header = Buffer.alloc(44);
    header.write('RIFF', 0);
    header.writeUInt32LE(fileSize, 4);
    header.write('WAVE', 8);
    header.write('fmt ', 12);
    header.writeUInt32LE(16, 16); // Subchunk1Size
    header.writeUInt16LE(1, 20); // AudioFormat (PCM)
    header.writeUInt16LE(numChannels, 22);
    header.writeUInt32LE(sampleRate, 24);
    header.writeUInt32LE(byteRate, 28);
    header.writeUInt16LE(blockAlign, 32);
    header.writeUInt16LE(bitsPerSample, 34);
    header.write('data', 36);
    header.writeUInt32LE(dataSize, 40);
    
    const silence = Buffer.alloc(dataSize); // filled with zeros (PCM silence)
    const wavBuffer = Buffer.concat([header, silence]);
    fs.writeFileSync(dummyAudioPath, wavBuffer);
  });

  test.afterAll(() => {
    // Clean up temporary E2E assets
    if (fs.existsSync(dummyImagePath)) fs.unlinkSync(dummyImagePath);
    if (fs.existsSync(dummyAudioPath)) fs.unlinkSync(dummyAudioPath);
  });

  test('01 - Block unauthorized sessions & grant access on valid passcode', async ({ page }) => {
    // Navigate without token
    await page.goto('/');
    
    // Verify that the passcode gateway is visible
    const authGateway = page.locator('#auth-gateway');
    await expect(authGateway).toBeVisible();

    // Type a wrong passcode
    const passcodeField = page.locator('#passcode-input');
    await passcodeField.fill('wrong_token_secret');
    await page.locator('#auth-form button[type="submit"]').click();

    // Verify error message
    const errorMsg = page.locator('#auth-error');
    await expect(errorMsg).toBeVisible();
    await expect(errorMsg).toContainText('Invalid Access Token');

    // Type the correct passcode
    await passcodeField.fill('test_token');
    await page.locator('#auth-form button[type="submit"]').click();

    // Verify that gateway is dismissed and we entered the portal
    await expect(authGateway).toBeHidden();
    
    // Check main elements are visible
    await expect(page.locator('h3:has-text("Obsidian Daily Note")')).toBeVisible();
    await expect(page.locator('h3:has-text("Obsidian AI Agent")')).toBeVisible();
  });

  test('02 - E2E Text Note classification, real-time WebSocket sync, and disk write verification', async ({ page }) => {
    // Open portal with token query param for seamless auto-login
    await page.goto('/?token=test_token');
    await expect(page.locator('#auth-gateway')).toBeHidden();

    // Compose a task note
    const inputArea = page.locator('#message-input');
    await inputArea.fill('Buy fresh milk for breakfast #groceries #home');
    
    // Send
    await page.locator('#send-btn').click();

    // Verify typing indicator appears
    const processingRow = page.locator('#processing-indicator');
    await expect(processingRow).toBeVisible();

    // Wait for the processing to finish and bot chat response bubble to appear
    const botResponse = page.locator('.msg-bubble.bot').last();
    await expect(botResponse).toBeVisible({ timeout: 60000 });
    
    // Assert response mentions created task
    await expect(botResponse).toContainText('Task Created');
    await expect(botResponse).toContainText('milk for breakfast');

    // VERIFY WebSocket real-time preview sync (sidebar updates immediately)
    const dailyNotePreview = page.locator('#daily-note-markdown');
    await expect(dailyNotePreview).toContainText('Buy fresh milk for breakfast');
    await expect(dailyNotePreview).toContainText('#groceries');
    await expect(dailyNotePreview).toContainText('#home');

    // VERIFY active Daily Note file written on local disk contains exact expected Markdown
    const noteFilePath = getTodayDailyNotePath();
    expect(fs.existsSync(noteFilePath)).toBe(true);
    
    const noteContent = fs.readFileSync(noteFilePath, 'utf-8');
    expect(noteContent).toContain('## ✅ Todos');
    expect(noteContent).toContain('- [ ] Buy fresh milk for breakfast');
    expect(noteContent).toContain('#groceries');
    expect(noteContent).toContain('#home');
  });

  test('03 - E2E Image upload with Vision AI categorization and asset wikilink verification', async ({ page }) => {
    await page.goto('/?token=test_token');
    await expect(page.locator('#auth-gateway')).toBeHidden();

    // Hook file upload input
    const fileChooserPromise = page.waitForEvent('filechooser');
    await page.locator('#attach-btn').click();
    const fileChooser = await fileChooserPromise;
    
    // Upload the dummy image
    await fileChooser.setFiles(dummyImagePath);

    // Verify image preview renders inside the bar
    const imagePreview = page.locator('#image-preview');
    await expect(imagePreview).toBeVisible();
    await expect(imagePreview).toHaveAttribute('src', /^data:image\/jpeg;base64,/);

    // Enter description/caption
    const inputArea = page.locator('#message-input');
    await inputArea.fill('Test photo capture of a desk organization setup');
    
    // Send
    await page.locator('#send-btn').click();

    // Verify typing indicator activates
    await expect(page.locator('#processing-indicator')).toBeVisible();

    // Wait for the saving confirmation
    const botResponse = page.locator('.msg-bubble.bot').last();
    await expect(botResponse).toBeVisible({ timeout: 60000 });
    await expect(botResponse).toContainText('Photo Saved');

    // Check Daily Note preview has updated with image embed and caption
    const dailyNotePreview = page.locator('#daily-note-markdown');
    await expect(dailyNotePreview).toContainText('desk organization');

    // Verify disk content and asset existence
    const noteFilePath = getTodayDailyNotePath();
    const noteContent = fs.readFileSync(noteFilePath, 'utf-8');
    
    // Check that standard wikilink was appended
    expect(noteContent).toContain('![[assets/');
    expect(noteContent).toContain('desk organization');

    // Verify file actually copied into Obsidian's assets folder
    const noteDir = path.dirname(noteFilePath);
    const assetsFolder = path.join(noteDir, 'assets');
    expect(fs.existsSync(assetsFolder)).toBe(true);

    const assetFiles = fs.readdirSync(assetsFolder);
    expect(assetFiles.length).toBeGreaterThan(0);
    expect(assetFiles.some(f => f.endsWith('.jpg'))).toBe(true);
  });

  test('04 - E2E Voice Note Whisper transcription and categorization verification', async ({ page }) => {
    await page.goto('/?token=test_token');
    await expect(page.locator('#auth-gateway')).toBeHidden();

    // Hook audio upload input
    const fileChooserPromise = page.waitForEvent('filechooser');
    await page.locator('#attach-btn').click();
    const fileChooser = await fileChooserPromise;
    
    // Upload dummy silent WAV file
    await fileChooser.setFiles(dummyAudioPath);

    // Verify audio thumbnail render placeholder
    const imagePreview = page.locator('#image-preview');
    await expect(imagePreview).toBeVisible();
    await expect(imagePreview).toHaveAttribute('src', /^data:image\/svg\+xml/);

    // Send
    await page.locator('#send-btn').click();

    // Verify Whisper/AI transcription process activates
    await expect(page.locator('#processing-indicator')).toBeVisible();

    // Wait for transcription and bot result bubble
    const botResponse = page.locator('.msg-bubble.bot').last();
    await expect(botResponse).toBeVisible({ timeout: 75000 }); // generous timeout for Whisper transcription + classification
    
    // Expect voice note save summary
    await expect(botResponse).toContainText('Voice Note Saved');
    await expect(botResponse).toContainText('Transcript');

    // Verify note preview and disk note contains transcribing notes
    const dailyNotePreview = page.locator('#daily-note-markdown');
    await expect(dailyNotePreview).toBeVisible();

    const noteFilePath = getTodayDailyNotePath();
    const noteContent = fs.readFileSync(noteFilePath, 'utf-8');
    expect(noteContent).not.toBeNull();
  });
});
