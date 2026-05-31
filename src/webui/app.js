// WebUI Client-side Application
document.addEventListener("DOMContentLoaded", () => {
    // Helper to safely execute lucide icon updates (robust against offline CDN load failure)
    function safeCreateIcons() {
        if (typeof lucide !== 'undefined' && lucide.createIcons) {
            try {
                lucide.createIcons();
            } catch (e) {
                console.error("Lucide icon generation failed:", e);
            }
        }
    }

    // Select elements
    const authGateway = document.getElementById("auth-gateway");
    const authForm = document.getElementById("auth-form");
    const passcodeInput = document.getElementById("passcode-input");
    const authError = document.getElementById("auth-error");

    const syncStatusText = document.getElementById("sync-status-text");
    const syncBadge = document.getElementById("sync-badge");
    const noteDateLabel = document.getElementById("note-date");
    const dailyNoteMarkdown = document.getElementById("daily-note-markdown");

    const logoutBtn = document.getElementById("logout-btn");
    const chatMessages = document.getElementById("chat-messages");
    const chatForm = document.getElementById("chat-form");
    const messageInput = document.getElementById("message-input");
    
    const fileUpload = document.getElementById("file-upload");
    const attachBtn = document.getElementById("attach-btn");
    
    const mediaPreviewContainer = document.getElementById("media-preview-container");
    const imagePreview = document.getElementById("image-preview");
    const clearPreviewBtn = document.getElementById("clear-preview-btn");
    
    const recordBtn = document.getElementById("record-btn");
    const micIcon = document.getElementById("mic-icon");
    const recordingOverlay = document.getElementById("recording-overlay");
    const recordingTimer = document.getElementById("recording-timer");
    const sendBtn = document.getElementById("send-btn");
    
    const processingIndicator = document.getElementById("processing-indicator");
    const processingText = document.getElementById("processing-text");

    // Runtime state
    let authToken = "";
    let socket = null;
    let reconnectTimeout = null;
    let activeFile = null;
    
    // Recording variables
    let mediaRecorder = null;
    let audioChunks = [];
    let recordInterval = null;
    let recordDuration = 0;

    // Initialize Lucide Icons
    safeCreateIcons();

    // Textarea Auto-grow helper
    messageInput.addEventListener("input", () => {
        messageInput.style.height = "auto";
        messageInput.style.height = (messageInput.scrollHeight - 4) + "px";
    });

    // 1. Session & Passcode Authentication Flow
    function checkSession() {
        // First check URL query parameter
        const urlParams = new URLSearchParams(window.location.search);
        const urlToken = urlParams.get("token");
        
        if (urlToken) {
            authToken = urlToken;
            localStorage.setItem("webui_token", urlToken);
            // Clean up the URL bar
            window.history.replaceState({}, document.title, window.location.pathname);
        } else {
            // Check localStorage
            authToken = localStorage.getItem("webui_token") || "";
        }

        if (authToken) {
            authGateway.classList.add("hidden");
            initWebSocket();
            fetchCurrentNote();
        } else {
            authGateway.classList.remove("hidden");
        }
    }

    authForm.addEventListener("submit", (e) => {
        e.preventDefault();
        const tokenVal = passcodeInput.value.trim();
        if (tokenVal) {
            authToken = tokenVal;
            // Verify token by making a quick API read call
            verifyAndSaveToken(tokenVal);
        }
    });

    async function verifyAndSaveToken(token) {
        setLoading(true, "Authorizing portal...");
        try {
            const res = await fetch("/api/note", {
                headers: { "Authorization": `Bearer ${token}` }
            });
            if (res.ok) {
                localStorage.setItem("webui_token", token);
                authToken = token; // Ensure global state is updated!
                authGateway.classList.add("hidden");
                initWebSocket();
                fetchCurrentNote();
                appendSystemMessage("🔒 Secure session successfully established.");
            } else if (res.status === 401) {
                showAuthError("Invalid Access Token. Please try again.");
            } else {
                const errText = await res.text();
                showAuthError(`Server error (${res.status}): ${errText}`);
            }
        } catch (err) {
            showAuthError(`Failed to authenticate: ${err.message}`);
        } finally {
            setLoading(false);
        }
    }

    function showAuthError(message = "Invalid Access Token. Please try again.") {
        authError.textContent = message;
        authError.classList.remove("hidden");
        passcodeInput.value = "";
        authToken = "";
        localStorage.removeItem("webui_token");
    }

    logoutBtn.addEventListener("click", () => {
        localStorage.removeItem("webui_token");
        authToken = "";
        if (socket) socket.close();
        authGateway.classList.remove("hidden");
        appendSystemMessage("🔒 Session terminated.");
    });

    // 2. WebSocket Real-time Updates Engine
    function initWebSocket() {
        if (socket) {
            socket.close();
        }

        const loc = window.location;
        const wsProto = loc.protocol === "https:" ? "wss:" : "ws:";
        const wsUrl = `${wsProto}//${loc.host}/ws?token=${encodeURIComponent(authToken)}`;

        updateSyncBadge("connecting", "Connecting...");
        socket = new WebSocket(wsUrl);

        socket.onopen = () => {
            console.log("WebSocket connection established");
            updateSyncBadge("idle", "Live Connected");
            if (reconnectTimeout) {
                clearTimeout(reconnectTimeout);
                reconnectTimeout = null;
            }
        };

        socket.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                if (data.type === "note_update") {
                    renderDailyNote(data.date, data.content);
                    updateSyncBadge("idle", "Synced just now");
                    setTimeout(() => updateSyncBadge("idle", "Live Connected"), 3000);
                } else if (data.type === "error") {
                    appendSystemMessage(`❌ Server Error: ${data.message}`);
                }
            } catch (e) {
                console.error("Failed to parse WebSocket message:", e);
            }
        };

        socket.onclose = (event) => {
            console.log("WebSocket closed:", event);
            updateSyncBadge("working", "Disconnected");
            
            if (event.code === 4001) {
                // Auth error - kick to login
                appendSystemMessage("❌ Session expired or invalid authorization.");
                logoutBtn.click();
                return;
            }

            // Retry connection
            if (!reconnectTimeout) {
                reconnectTimeout = setTimeout(() => {
                    reconnectTimeout = null;
                    initWebSocket();
                }, 5000);
            }
        };

        socket.onerror = (err) => {
            console.error("WebSocket error:", err);
            updateSyncBadge("working", "Connection error");
        };
    }

    async function fetchCurrentNote() {
        try {
            const res = await fetch("/api/note", {
                headers: { "Authorization": `Bearer ${authToken}` }
            });
            if (res.ok) {
                const data = await res.json();
                renderDailyNote(data.date, data.content);
            } else if (res.status === 401) {
                console.warn("Unauthorized token detected in fetchCurrentNote, forcing logout.");
                logoutBtn.click();
            }
        } catch (e) {
            console.error("Failed to fetch daily note:", e);
        }
    }

    function updateSyncBadge(state, text) {
        syncStatusText.textContent = text;
        syncBadge.className = "badge";
        
        if (state === "idle") {
            syncBadge.classList.add("idle");
        } else if (state === "working") {
            syncBadge.classList.add("working");
        } else if (state === "connecting") {
            syncBadge.classList.add("working");
        }
    }

    // 3. Regex-based Beautiful Markdown Parser
    function renderDailyNote(date, markdown) {
        noteDateLabel.textContent = date;
        if (!markdown || markdown.trim() === "") {
            dailyNoteMarkdown.innerHTML = `<p class="placeholder-text">Today's Daily Note is empty. Use the chat to add logs, tasks, and notes!</p>`;
            return;
        }

        // Clean parse
        let html = markdown;

        // Escape HTML tags to prevent XSS
        html = html.replace(/</g, "&lt;").replace(/>/g, "&gt;");

        // Restore blockquotes after escape
        html = html.replace(/^&gt;\s*(.*)$/gm, "<blockquote>$1</blockquote>");
        // Combine nested blockquotes
        html = html.replace(/<\/blockquote>\n<blockquote>/g, "<br>");

        // Headers
        html = html.replace(/^#\s*(.*)$/gm, "<h1>$1</h1>");
        html = html.replace(/^##\s*(.*)$/gm, "<h2>$1</h2>");
        html = html.replace(/^###\s*(.*)$/gm, "<h3>$1</h3>");

        // Completed Todos
        html = html.replace(/^-\s*\[x\]\s*(.*)$/gm, '<li><input type="checkbox" checked disabled> <span class="strikethrough">$1</span></li>');
        // Uncompleted Todos
        html = html.replace(/^-\s*\[\s*\]\s*(.*)$/gm, '<li><input type="checkbox" disabled> <span>$1</span></li>');
        // Raw list items
        html = html.replace(/^-\s*(?!\[)(.*)$/gm, "<li>$1</li>");

        // Bold & Italic
        html = html.replace(/\*\*(.*?)\*\*/g, "<strong>$1</strong>");
        html = html.replace(/\*(.*?)\*/g, "<em>$1</em>");
        
        // Inline Code
        html = html.replace(/`(.*?)`/g, "<code>$1</code>");

        // Handle Obsidian Wikilinked images: ![[assets/filename.jpg]] -> map to /assets/filename.jpg
        html = html.replace(/!\[\[(?:assets\/)?(.*?)\]\]/g, (match, filename) => {
            return `<img src="/assets/${encodeURIComponent(filename)}" alt="${filename}" class="note-embedded-image">`;
        });
        
        // Render general wikilinks: [[link]] -> link text
        html = html.replace(/\[\[(.*?)\]\]/g, '<span class="wikilink">$1</span>');

        // Wrap list items nicely
        html = html.replace(/(<li>.*?<\/li>)/g, '<ul class="note-list">$1</ul>');
        html = html.replace(/<\/ul>\n<ul class="note-list">/g, ""); // Collapse adjacent lists

        dailyNoteMarkdown.innerHTML = html;
    }

    // 4. Client REST Operations (Sending Text, Photos, Voice)
    chatForm.addEventListener("submit", async (e) => {
        e.preventDefault();
        
        const messageText = messageInput.value.trim();
        
        if (!messageText && !activeFile) return;

        // Reset input fields
        messageInput.value = "";
        messageInput.style.height = "auto";

        // Show typing indicator
        setLoading(true, "AI is processing note...");

        try {
            if (activeFile) {
                // Check if file is image or audio
                const fileType = activeFile.type;
                const formData = new FormData();
                formData.append("file", activeFile);
                if (messageText) {
                    formData.append("caption", messageText);
                }

                // Add message bubble for user
                if (fileType.startsWith("image/")) {
                    appendUserMessage(messageText || "Uploaded an image", imagePreview.src);
                    clearPreview();

                    const res = await fetch("/api/photo", {
                        method: "POST",
                        headers: { "Authorization": `Bearer ${authToken}` },
                        body: formData
                    });
                    
                    if (res.ok) {
                        const data = await res.json();
                        appendBotMessage(`📸 **Photo Saved Successfully!**\n*Filename*: \`${data.filename}\`\n*AI Summary*: "${data.summary}"`);
                        fetchCurrentNote();
                    } else {
                        throw new Error(await res.text() || "Failed to upload image");
                    }
                } else if (fileType === "application/pdf" || activeFile.name.endsWith(".pdf")) {
                    appendUserMessage(`📄 Sent PDF: ${activeFile.name}`);
                    clearPreview();

                    const res = await fetch("/api/pdf", {
                        method: "POST",
                        headers: { "Authorization": `Bearer ${authToken}` },
                        body: formData
                    });
                    
                    if (res.ok) {
                        const data = await res.json();
                        if (data.gemini_success) {
                            appendBotMessage(`📄 **PDF Document Logged Successfully!**\n\n**Title**: ${data.title}\n*PDF File*: \`${data.pdf_filename}\`\n*Transcript File*: \`${data.transcript_filename}\`\n\nAppended to Daily Note logs.`);
                        } else {
                            appendBotMessage(`⚠️ **PDF Saved (Transcription Skipped)**\n\n*Original PDF File*: \`${data.pdf_filename}\`\n\n*Note*: Gemini client is not configured or transcription failed. Only the original document reference has been logged.`);
                        }
                        fetchCurrentNote();
                    } else {
                        throw new Error(await res.text() || "Failed to upload PDF");
                    }
                } else if (fileType.startsWith("audio/") || activeFile.name.endsWith(".wav") || activeFile.name.endsWith(".mp3") || activeFile.name.endsWith(".m4a") || activeFile.name.endsWith(".ogg") || activeFile.name.endsWith(".webm")) {
                    appendUserMessage("🎙️ Sent a voice note");
                    clearPreview();

                    const res = await fetch("/api/voice", {
                        method: "POST",
                        headers: { "Authorization": `Bearer ${authToken}` },
                        body: formData
                    });
                    
                    if (res.ok) {
                        const data = await res.json();
                        appendBotMessage(`🎙️ **Voice Note Saved Successfully!**\n*Category*: \`${data.category}\`\n*Summary*: "${data.summary}"\n\n**Transcript**: "${data.transcript}"`);
                        fetchCurrentNote();
                    } else {
                        throw new Error(await res.text() || "Failed to transcribe audio");
                    }
                }
            } else {
                // Text Message
                appendUserMessage(messageText);
                
                const res = await fetch("/api/message", {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json",
                        "Authorization": `Bearer ${authToken}`
                    },
                    body: JSON.stringify({ text: messageText })
                });

                if (res.ok) {
                    const data = await res.json();
                    let responseMsg = "";
                    if (data.category === "todo") {
                        responseMsg = `✅ **Task Created** under **Todo** category!\n*Summary*: "${data.summary}"\n*Tags*: ${data.tags.map(t => `#${t}`).join(" ") || "None"}`;
                    } else if (data.category === "note") {
                        responseMsg = `📝 **Note Captured** and categorized successfully.\n*Summary*: "${data.summary}"\n*Tags*: ${data.tags.map(t => `#${t}`).join(" ") || "None"}`;
                    } else {
                        responseMsg = `👍 **Log Appended** as raw entry to your daily notes list.\n*Summary*: "${data.summary}"`;
                    }
                    
                    if (!data.ai_success) {
                        responseMsg = `📝 **Appended Raw Entry** (AI classification failed or returned raw fallback).\n*Entry*: "${data.summary}"`;
                    }

                    appendBotMessage(responseMsg);
                    fetchCurrentNote();
                } else {
                    throw new Error(await res.text() || "Failed to process message");
                }
            }
        } catch (err) {
            console.error(err);
            appendSystemMessage(`❌ Operation failed: ${err.message}`);
        } finally {
            setLoading(false);
        }
    });

    // 5. File uploads & Previews handling
    attachBtn.addEventListener("click", () => fileUpload.click());

    fileUpload.addEventListener("change", (e) => {
        const file = e.target.files[0];
        if (!file) return;

        activeFile = file;

        // Render image previews in the input bar
        if (file.type.startsWith("image/")) {
            const reader = new FileReader();
            reader.onload = (event) => {
                imagePreview.src = event.target.result;
                mediaPreviewContainer.classList.remove("hidden");
                messageInput.focus();
            };
            reader.readAsDataURL(file);
        } else if (file.type === "application/pdf" || file.name.endsWith(".pdf")) {
            // PDF preview state
            imagePreview.src = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100' viewBox='0 0 24 24' fill='none' stroke='%23ef4444' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z'></path><polyline points='14 2 14 8 20 8'></polyline><path d='M8 13h8'></path><path d='M8 17h8'></path><path d='M10 9H8'></path></svg>";
            mediaPreviewContainer.classList.remove("hidden");
            messageInput.focus();
        } else {
            // Audio preview state
            imagePreview.src = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100' viewBox='0 0 24 24' fill='none' stroke='%238b5cf6' stroke-width='2'><path d='M9 18V5l12-2v13'></path><circle cx='6' cy='18' r='3'></circle><circle cx='18' cy='16' r='3'></circle></svg>";
            mediaPreviewContainer.classList.remove("hidden");
            messageInput.focus();
        }
    });

    clearPreviewBtn.addEventListener("click", clearPreview);

    function clearPreview() {
        mediaPreviewContainer.classList.add("hidden");
        imagePreview.src = "#";
        fileUpload.value = "";
        activeFile = null;
    }

    // 6. Voice Recording Engine (MediaRecorder browser APIs)
    recordBtn.addEventListener("click", toggleRecording);

    async function toggleRecording() {
        if (mediaRecorder && mediaRecorder.state === "recording") {
            // Stop recording
            mediaRecorder.stop();
            return;
        }

        // Start Recording
        if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
            appendSystemMessage("❌ Microphone recording is not supported in this browser.");
            return;
        }

        try {
            const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
            audioChunks = [];
            
            // Use WebM or fallback container
            let options = { mimeType: 'audio/webm' };
            if (!MediaRecorder.isTypeSupported('audio/webm')) {
                options = { mimeType: 'audio/ogg' };
            }
            if (!MediaRecorder.isTypeSupported('audio/ogg')) {
                options = {}; // use browser default
            }

            mediaRecorder = new MediaRecorder(stream, options);
            
            mediaRecorder.ondataavailable = (event) => {
                if (event.data.size > 0) {
                    audioChunks.push(event.data);
                }
            };

            mediaRecorder.onstop = async () => {
                // Stop UI updates
                clearInterval(recordInterval);
                recordBtn.classList.remove("mic-active");
                recordingOverlay.classList.add("hidden");
                messageInput.classList.remove("hidden");
                attachBtn.classList.remove("hidden");
                sendBtn.classList.remove("hidden");
                micIcon.setAttribute("data-lucide", "mic");
                safeCreateIcons();

                // Build audio Blob
                const audioBlob = new Blob(audioChunks, { type: mediaRecorder.mimeType || 'audio/webm' });
                
                // Create audio file
                const fileExt = mediaRecorder.mimeType.includes("ogg") ? "ogg" : "webm";
                activeFile = new File([audioBlob], `voice-recording.${fileExt}`, { type: audioBlob.type });

                // Direct instant send
                chatForm.dispatchEvent(new Event("submit"));

                // Stop tracks
                stream.getTracks().forEach(track => track.stop());
                mediaRecorder = null;
            };

            // Start UI timer updates
            recordDuration = 0;
            recordingTimer.textContent = "00:00";
            recordBtn.classList.add("mic-active");
            recordingOverlay.classList.remove("hidden");
            messageInput.classList.add("hidden");
            attachBtn.classList.add("hidden");
            sendBtn.classList.add("hidden");
            
            micIcon.setAttribute("data-lucide", "square");
            safeCreateIcons();

            recordInterval = setInterval(() => {
                recordDuration += 1;
                const mins = String(Math.floor(recordDuration / 60)).padStart(2, '0');
                const secs = String(recordDuration % 60).padStart(2, '0');
                recordingTimer.textContent = `${mins}:${secs}`;
            }, 1000);

            mediaRecorder.start();

        } catch (err) {
            console.error("Failed to start recording:", err);
            appendSystemMessage(`❌ Microphone access denied: ${err.message}`);
        }
    }

    // 7. Chat Render Helpers
    function appendUserMessage(text, imgUrl = null) {
        const bubble = document.createElement("div");
        bubble.className = "msg-bubble user";
        
        let innerHtml = '<div class="msg-content">';
        if (imgUrl) {
            innerHtml += `<img src="${imgUrl}" class="msg-attachment-img" alt="Attachment">`;
        }
        if (text) {
            innerHtml += `<p>${escapeHTML(text)}</p>`;
        }
        innerHtml += `<div class="msg-meta">${getFormattedTime()}</div></div>`;
        
        bubble.innerHTML = innerHtml;
        chatMessages.appendChild(bubble);
        scrollToBottom();
    }

    function appendBotMessage(markdown) {
        const bubble = document.createElement("div");
        bubble.className = "msg-bubble bot";
        
        // Simple parser for chat responses
        let text = markdown;
        text = text.replace(/\*\*(.*?)\*\*/g, "<strong>$1</strong>");
        text = text.replace(/\*(.*?)\*/g, "<em>$1</em>");
        text = text.replace(/`(.*?)`/g, "<code>$1</code>");
        text = text.replace(/\n/g, "<br>");

        bubble.innerHTML = `
            <div class="msg-content">
                <p>${text}</p>
                <div class="msg-meta">${getFormattedTime()}</div>
            </div>
        `;
        
        chatMessages.appendChild(bubble);
        scrollToBottom();
    }

    function appendSystemMessage(text) {
        const bubble = document.createElement("div");
        bubble.className = "msg-bubble system";
        bubble.innerHTML = `
            <div class="msg-content">
                <p>${escapeHTML(text)}</p>
                <div class="msg-meta">${getFormattedTime()}</div>
            </div>
        `;
        chatMessages.appendChild(bubble);
        scrollToBottom();
    }

    function setLoading(isLoading, text = "AI is thinking...") {
        if (isLoading) {
            processingText.textContent = text;
            processingIndicator.classList.remove("hidden");
        } else {
            processingIndicator.classList.add("hidden");
        }
    }

    function scrollToBottom() {
        chatMessages.scrollTop = chatMessages.scrollHeight;
    }

    function escapeHTML(str) {
        return str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
    }

    function getFormattedTime() {
        const now = new Date();
        const hrs = String(now.getHours()).padStart(2, '0');
        const mins = String(now.getMinutes()).padStart(2, '0');
        return `${hrs}:${mins}`;
    }

    // 8. Run Setup On Startup
    checkSession();
});
