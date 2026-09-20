DELETE FROM tokens
 WHERE chain_id = 'eip155:11155111'
   AND lower(address) = lower('0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238');
