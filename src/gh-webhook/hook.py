#!/usr/bin/env python3

from flask import Flask, request, jsonify
import subprocess
import hmac
import hashlib
import os

app = Flask(__name__)

# Secret for verifying GitHub webhook signature
GITHUB_SECRET = 'bzpEZb1N6LY5O2woay7QB0NtKVXiSo2O'

# Check if Rust binary should be used
USE_RUST = os.environ.get('USE_RUST', '0') == '1'

def verify_signature(payload_body, signature_header):
    """Verify that the payload was sent from GitHub by validating SHA256.
       Raise and return 403 if not authorized.
    """
    if not signature_header:
        return False
    hash_object = hmac.new(GITHUB_SECRET.encode('utf-8'), msg=payload_body, digestmod=hashlib.sha256)
    expected_signature = "sha256=" + hash_object.hexdigest()
    return hmac.compare_digest(expected_signature, signature_header)

@app.route('/webhook', methods=['POST'])
def webhook():
    # Verify the request signature
    signature = request.headers.get('X-Hub-Signature-256')
    if not verify_signature(request.data, signature):
        return jsonify({'error': 'Invalid signature'}), 403

    # Process the webhook payload
    event = request.headers.get('X-GitHub-Event')
    payload = request.json

    if event == 'push':
        # Run check-push script (Rust or bash version)
        if USE_RUST:
            script_path = '/scripts/check-push-rs'
        else:
            script_path = '/scripts/check-push.sh'

        try:
            subprocess.run([script_path] + ['--once'], check=True)
            return jsonify({'status': 'CI job started', 'version': 'rust' if USE_RUST else 'bash'}), 200
        except subprocess.CalledProcessError as e:
            return jsonify({'error': f'Script execution failed: {e}'}), 500

    return jsonify({'status': 'Event not handled'}), 200

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=9870, debug=False)
