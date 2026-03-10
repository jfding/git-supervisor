#!/usr/bin/env python3

from flask import Flask, request, jsonify
import subprocess
import hmac
import hashlib
import os

app = Flask(__name__)

# Secret for verifying GitHub webhook signature
GITHUB_SECRET = 'bzpEZb1N6LY5O2woay7QB0NtKVXiSo2O'

def _app_version():
    """Read project version from VERSION file (in container or repo root)."""
    for path in ('/scripts/VERSION', '/gh-webhook/VERSION', 'VERSION'):
        try:
            with open(path) as f:
                return f.read().strip()
        except OSError:
            continue
    return None

def verify_signature(payload_body, signature_header):
    """Verify that the payload was sent from GitHub by validating SHA256.
       Raise and return 403 if not authorized.
    """
    if not signature_header:
        return False
    hash_object = hmac.new(GITHUB_SECRET.encode('utf-8'), msg=payload_body, digestmod=hashlib.sha256)
    expected_signature = "sha256=" + hash_object.hexdigest()
    return hmac.compare_digest(expected_signature, signature_header)

@app.route('/version', methods=['GET'])
def version():
    """Return the project version (from VERSION file)."""
    v = _app_version()
    return jsonify({'version': v or 'unknown'}), 200


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
        # Run check-push script
        script_path = '/scripts/check-push.sh'

        try:
            subprocess.run([script_path] + ['--once'], check=True)
            version = _app_version()
            payload = {'status': 'CI job started', 'engine': 'check-push.sh'}
            if version:
                payload['version'] = version
            return jsonify(payload), 200
        except subprocess.CalledProcessError as e:
            return jsonify({'error': f'Script execution failed: {e}'}), 500

    return jsonify({'status': 'Event not handled'}), 200

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=9870, debug=False)
