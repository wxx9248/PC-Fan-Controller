idf_component_register(
    SRCS
        "main.c"
        "fan_pwm.c"
        "tach.c"
        "failsafe.c"
        "fan_status.c"
        "protocol.c"
        "wifi_mgr.c"
        "http_server.c"
    INCLUDE_DIRS "."
    PRIV_REQUIRES
        esp_driver_gpio
        esp_driver_ledc
        esp_driver_usb_serial_jtag
        esp_timer
        esp_wifi
        esp_netif
        esp_event
        esp_http_server
        nvs_flash
    EMBED_FILES "web/index.html.gz"
)
