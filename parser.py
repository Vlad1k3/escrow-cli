import json
import sys
import base58

# Укажи путь к JSON-файлу
file_path = "arbiter.json"  # замените на путь к вашему файлу

# Загрузка массива ключей
with open(file_path, 'r') as f:
    secret_key = json.load(f)

# Преобразуем в байты
secret_key_bytes = bytes(secret_key)

# Выводим приватный ключ в base58
private_key_base58 = base58.b58encode(secret_key_bytes).decode('utf-8')
print("Private Key (base58):", private_key_base58)

# Если нужно в hex:
private_key_hex = secret_key_bytes.hex()
print("Private Key (hex):", private_key_hex)
