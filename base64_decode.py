#!/usr/bin/env python3

import base64
import sys

def convert_input(input_text):
    if input_text.startswith('sha256-'):
        decoded_text = base64.b64decode(input_text[7:])
        return decoded_text.hex()
    elif input_text.startswith('sha256:'):
        return input_text[7:]
    else:
        return base64.b64decode(input_text).hex()

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python3 base64_decode.py <input_text>")
        sys.exit(1)

    input_text = sys.argv[1]
    result = convert_input(input_text)
    print(result)
