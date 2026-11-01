//! Explicit UI messages. Device names, paths and technical error details are never translated.
use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    Russian,
    French,
    German,
    Spanish,
    Italian,
}

impl Language {
    pub const ALL: [Self; 6] = [
        Self::English,
        Self::Russian,
        Self::French,
        Self::German,
        Self::Spanish,
        Self::Italian,
    ];
    pub fn name(self) -> &'static str {
        [
            "English",
            "Русский",
            "Français",
            "Deutsch",
            "Español",
            "Italiano",
        ][self as usize]
    }
    pub fn code(self) -> &'static str {
        ["en", "ru", "fr", "de", "es", "it"][self as usize]
    }
    pub fn from_code(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == value)
    }
    pub fn text<'a>(self, key: &'a str) -> &'a str {
        MESSAGES
            .iter()
            .find(|row| row[0] == key)
            .map_or(key, |row| row[self as usize])
    }
}

thread_local! { static CURRENT: Cell<Language> = const { Cell::new(Language::English) }; }
pub fn language() -> Language {
    CURRENT.get()
}
pub fn set_language(language: Language) {
    CURRENT.set(language);
}
pub fn tr(key: &str) -> &str {
    language().text(key)
}
pub fn message(key: &str, name: &str, value: &str) -> String {
    tr(key).replace(name, value)
}

/// Only application-owned status templates are interpreted; arbitrary diagnostics stay intact.
pub fn status(text: &str) -> std::borrow::Cow<'_, str> {
    let translated = tr(text);
    if translated != text {
        return std::borrow::Cow::Borrowed(translated);
    }
    for (prefix, key) in [
        ("Tray unavailable: ", "Tray unavailable"),
        (
            "Could not enumerate devices: ",
            "Could not enumerate devices",
        ),
        ("Could not read settings: ", "Could not read settings"),
        ("Could not save settings: ", "Could not save settings"),
        (
            "Could not save audio settings before setup: ",
            "Could not save audio settings before setup",
        ),
        (
            "Could not confirm update startup: ",
            "Could not confirm update startup",
        ),
    ] {
        if let Some(detail) = text.strip_prefix(prefix) {
            return std::borrow::Cow::Owned(format!("{}: {detail}", tr(key)));
        }
    }
    if let Some(browser) = text
        .strip_prefix("Waiting for ")
        .and_then(|v| v.strip_suffix(" audio"))
    {
        return std::borrow::Cow::Owned(message(
            "Waiting for {browser} audio",
            "{browser}",
            browser,
        ));
    }
    if let Some(browser) = text.strip_suffix(" routed to Source") {
        return std::borrow::Cow::Owned(message(
            "{browser} routed to Source",
            "{browser}",
            browser,
        ));
    }
    std::borrow::Cow::Borrowed(text)
}

const MESSAGES: &[[&str; 6]] = &[
    ["Language", "Язык", "Langue", "Sprache", "Idioma", "Lingua"],
    ["Theme", "Тема", "Thème", "Design", "Tema", "Tema"],
    ["Light", "Светлая", "Clair", "Hell", "Claro", "Chiaro"],
    ["Dark", "Тёмная", "Sombre", "Dunkel", "Oscuro", "Scuro"],
    [
        "Settings",
        "Настройки",
        "Paramètres",
        "Einstellungen",
        "Ajustes",
        "Impostazioni",
    ],
    [
        "Sound controls",
        "Управление звуком",
        "Réglages du son",
        "Klangsteuerung",
        "Controles de sonido",
        "Controlli audio",
    ],
    [
        "Sound Lock — Settings",
        "Sound Lock — Настройки",
        "Sound Lock — Paramètres",
        "Sound Lock — Einstellungen",
        "Sound Lock — Ajustes",
        "Sound Lock — Impostazioni",
    ],
    [
        "Sound Lock — Sound controls",
        "Sound Lock — Управление звуком",
        "Sound Lock — Réglages du son",
        "Sound Lock — Klangsteuerung",
        "Sound Lock — Controles de sonido",
        "Sound Lock — Controlli audio",
    ],
    [
        "Choose an output in Settings",
        "Выберите выход в настройках",
        "Choisir une sortie dans les paramètres",
        "Ausgabe in Einstellungen wählen",
        "Elige una salida en Ajustes",
        "Scegli un'uscita nelle impostazioni",
    ],
    [
        "Dismiss · Esc",
        "Закрыть · Esc",
        "Masquer · Échap",
        "Schließen · Esc",
        "Ocultar · Esc",
        "Nascondi · Esc",
    ],
    [
        "Update",
        "Обновить",
        "Mettre à jour",
        "Aktualisieren",
        "Actualizar",
        "Aggiorna",
    ],
    [
        "System limiter",
        "Системный лимитер",
        "Limiteur système",
        "Systembegrenzer",
        "Limitador del sistema",
        "Limitatore di sistema",
    ],
    [
        "Boost + leveling",
        "Усиление + выравнивание",
        "Gain + égalisation",
        "Verstärkung + Ausgleich",
        "Ganancia + nivelación",
        "Guadagno + livellamento",
    ],
    [
        "Peak limit",
        "Предел пиков",
        "Limite des crêtes",
        "Spitzenpegel",
        "Límite de picos",
        "Limite dei picchi",
    ],
    [
        "Volume boost",
        "Усиление громкости",
        "Gain de volume",
        "Lautstärkeverstärkung",
        "Ganancia de volumen",
        "Guadagno del volume",
    ],
    ["Night", "Ночь", "Nuit", "Nacht", "Noche", "Notte"],
    [
        "Headphones",
        "Наушники",
        "Casque",
        "Kopfhörer",
        "Auriculares",
        "Cuffie",
    ],
    [
        "Speakers",
        "Колонки",
        "Haut-parleurs",
        "Lautsprecher",
        "Altavoces",
        "Altoparlanti",
    ],
    [
        "Softer peaks, gentle recovery.",
        "Мягкие пики и плавное восстановление.",
        "Crêtes atténuées, reprise douce.",
        "Sanftere Spitzen, langsame Erholung.",
        "Picos suaves y recuperación gradual.",
        "Picchi attenuati e recupero graduale.",
    ],
    [
        "Balanced dynamics, smooth recovery.",
        "Сбалансированная динамика и плавное восстановление.",
        "Dynamique équilibrée, reprise fluide.",
        "Ausgewogene Dynamik, sanfte Erholung.",
        "Dinámica equilibrada y recuperación suave.",
        "Dinamica bilanciata e recupero fluido.",
    ],
    [
        "Faster recovery, more room for dynamics.",
        "Быстрое восстановление и больше динамики.",
        "Reprise rapide, dynamique préservée.",
        "Schnellere Erholung, mehr Dynamik.",
        "Recuperación rápida y mayor dinámica.",
        "Recupero rapido e più dinamica.",
    ],
    [
        "Peak reduction",
        "Снижение пиков",
        "Réduction des crêtes",
        "Spitzenabsenkung",
        "Reducción de picos",
        "Riduzione dei picchi",
    ],
    [
        "Leveling",
        "Выравнивание",
        "Égalisation",
        "Pegelausgleich",
        "Nivelación",
        "Livellamento",
    ],
    ["Input", "Вход", "Entrée", "Eingang", "Entrada", "Ingresso"],
    [
        "Current gain reduction",
        "Текущее снижение усиления",
        "Réduction du gain actuelle",
        "Aktuelle Pegelabsenkung",
        "Reducción de ganancia actual",
        "Riduzione del guadagno attuale",
    ],
    [
        "Source",
        "Источник",
        "Source",
        "Quelle",
        "Fuente",
        "Sorgente",
    ],
    [
        "Listening",
        "Прослушивание",
        "Écoute",
        "Wiedergabe",
        "Escucha",
        "Ascolto",
    ],
    [
        "Choose a source output",
        "Выберите выход источника",
        "Choisir une sortie source",
        "Quellausgabe wählen",
        "Elige una salida de origen",
        "Scegli un'uscita sorgente",
    ],
    [
        "Choose headphones or speakers",
        "Выберите наушники или колонки",
        "Choisir un casque ou des haut-parleurs",
        "Kopfhörer oder Lautsprecher wählen",
        "Elige auriculares o altavoces",
        "Scegli cuffie o altoparlanti",
    ],
    [
        "Refresh devices",
        "Обновить устройства",
        "Actualiser les appareils",
        "Geräte aktualisieren",
        "Actualizar dispositivos",
        "Aggiorna dispositivi",
    ],
    [
        "Auto-route",
        "Автомаршрут",
        "Routage auto",
        "Automatisch zuweisen",
        "Enrutamiento auto",
        "Instradamento auto",
    ],
    [
        "Switch this browser to Source on Start; restore its previous output on Stop/Quit.",
        "При запуске направлять браузер в источник; при остановке или выходе возвращать прежний выход.",
        "Au démarrage, diriger ce navigateur vers Source ; à l'arrêt ou à la fermeture, restaurer sa sortie.",
        "Browser beim Start der Quelle zuweisen; beim Stoppen oder Beenden die vorherige Ausgabe wiederherstellen.",
        "Envía este navegador a Fuente al iniciar; restaura su salida al detener o salir.",
        "Invia questo browser alla sorgente all'avvio; ripristina l'uscita precedente all'arresto o all'uscita.",
    ],
    [
        "Stop processing before changing devices.",
        "Остановите обработку перед сменой устройств.",
        "Arrêtez le traitement avant de changer d'appareil.",
        "Verarbeitung vor dem Gerätewechsel stoppen.",
        "Detén el procesamiento antes de cambiar dispositivos.",
        "Arresta l'elaborazione prima di cambiare dispositivo.",
    ],
    [
        "Driver-free peak reduction using Windows volume. Stop restores your starting level. Choose Boost + leveling for full processing and 400% gain.",
        "Снижение пиков через громкость Windows без драйвера. Остановка возвращает исходный уровень. Для полной обработки и усиления до 400% выберите «Усиление + выравнивание».",
        "Réduction des crêtes via le volume Windows, sans pilote. L'arrêt restaure le niveau initial. Choisissez Gain + égalisation pour le traitement complet et un gain de 400 %.",
        "Spitzenabsenkung über die Windows-Lautstärke ohne Treiber. Stoppen stellt den Ausgangspegel wieder her. Verstärkung + Ausgleich bietet volle Verarbeitung und bis zu 400 % Verstärkung.",
        "Reduce picos con el volumen de Windows, sin controlador. Detener restaura el nivel inicial. Usa Ganancia + nivelación para el procesamiento completo y hasta un 400 %.",
        "Riduce i picchi tramite il volume di Windows, senza driver. L'arresto ripristina il livello iniziale. Scegli Guadagno + livellamento per l'elaborazione completa e un guadagno del 400%.",
    ],
    [
        "Keep Source unmuted and inaudible. Start routes the browser here; Stop/Quit restores its previous output. Play audio if waiting for the browser.",
        "Источник должен быть включён, но не слышен. Запуск направляет сюда браузер; остановка или выход возвращает прежний выход. Если браузер ожидается, включите звук в нём.",
        "Gardez Source active mais inaudible. Démarrer y dirige le navigateur ; Arrêter ou Quitter restaure sa sortie. Lancez un son si le navigateur est en attente.",
        "Quelle nicht stummschalten, aber unhörbar halten. Start weist den Browser hier zu; Stoppen/Beenden stellt seine Ausgabe wieder her. Bei Warteanzeige Audio abspielen.",
        "Mantén Fuente activa pero inaudible. Iniciar dirige aquí el navegador; Detener/Salir restaura su salida. Reproduce audio si se espera al navegador.",
        "Mantieni la sorgente attiva ma non udibile. Avvia vi invia il browser; Arresta/Esci ripristina l'uscita. Riproduci audio se il browser è in attesa.",
    ],
    [
        "Route the browser to Source manually in Windows Volume Mixer. Keep Source unmuted and inaudible, and restore the browser output after stopping.",
        "В микшере Windows вручную направьте браузер в источник. Источник должен быть включён, но не слышен. После остановки верните выход браузера.",
        "Dirigez le navigateur vers Source dans le mélangeur Windows. Gardez Source active mais inaudible, puis restaurez la sortie après l'arrêt.",
        "Browser im Windows-Lautstärkemixer manuell der Quelle zuweisen. Quelle nicht stummschalten, aber unhörbar halten. Nach dem Stoppen die Browserausgabe wiederherstellen.",
        "Dirige el navegador a Fuente en el mezclador de Windows. Mantén Fuente activa pero inaudible y restaura la salida al detener.",
        "Invia manualmente il browser alla sorgente nel mixer di Windows. Mantienila attiva ma non udibile e ripristina l'uscita del browser dopo l'arresto.",
    ],
    [
        "Windows volume mixer",
        "Микшер громкости Windows",
        "Mélangeur de volume Windows",
        "Windows-Lautstärkemixer",
        "Mezclador de volumen de Windows",
        "Mixer volume di Windows",
    ],
    [
        "Detected",
        "Обнаружен",
        "Détecté",
        "Erkannt",
        "Detectado",
        "Rilevato",
    ],
    [
        "Not detected",
        "Не обнаружен",
        "Non détecté",
        "Nicht erkannt",
        "No detectado",
        "Non rilevato",
    ],
    [
        "Optional virtual output for full leveling and 400% gain without a spare physical output.",
        "Дополнительный виртуальный выход для выравнивания и усиления до 400% без запасного физического выхода.",
        "Sortie virtuelle facultative pour l'égalisation et un gain de 400 %, sans sortie physique supplémentaire.",
        "Optionale virtuelle Ausgabe für Pegelausgleich und 400 % Verstärkung ohne zusätzliche physische Ausgabe.",
        "Salida virtual opcional para nivelación y ganancia del 400 % sin otra salida física.",
        "Uscita virtuale facoltativa per livellamento e guadagno del 400% senza un'altra uscita fisica.",
    ],
    [
        "Downloading…",
        "Загрузка…",
        "Téléchargement…",
        "Wird heruntergeladen…",
        "Descargando…",
        "Download…",
    ],
    [
        "Download driver",
        "Скачать драйвер",
        "Télécharger le pilote",
        "Treiber herunterladen",
        "Descargar controlador",
        "Scarica driver",
    ],
    [
        "Installer open…",
        "Установщик открыт…",
        "Installateur ouvert…",
        "Installationsprogramm geöffnet…",
        "Instalador abierto…",
        "Programma di installazione aperto…",
    ],
    [
        "Install driver…",
        "Установить драйвер…",
        "Installer le pilote…",
        "Treiber installieren…",
        "Instalar controlador…",
        "Installa driver…",
    ],
    [
        "Official website & license",
        "Официальный сайт и лицензия",
        "Site officiel et licence",
        "Offizielle Website und Lizenz",
        "Sitio oficial y licencia",
        "Sito ufficiale e licenza",
    ],
    [
        "Installation opens the vendor's setup with Windows UAC. Administrator access and a Windows restart are required.",
        "Установка открывает программу производителя с запросом UAC. Нужны права администратора и перезагрузка Windows.",
        "L'installation ouvre le programme du fournisseur avec l'UAC. Des droits administrateur et un redémarrage de Windows sont nécessaires.",
        "Die Installation öffnet das Herstellerprogramm mit Windows-UAC. Administratorrechte und ein Windows-Neustart sind erforderlich.",
        "La instalación abre el programa del proveedor con UAC. Requiere permisos de administrador y reiniciar Windows.",
        "L'installazione apre il programma del produttore con UAC. Sono necessari diritti di amministratore e un riavvio di Windows.",
    ],
    [
        "Stop processing before installing or changing the route.",
        "Остановите обработку перед установкой или сменой маршрута.",
        "Arrêtez le traitement avant l'installation ou le changement de routage.",
        "Verarbeitung vor Installation oder Routenänderung stoppen.",
        "Detén el procesamiento antes de instalar o cambiar la ruta.",
        "Arresta l'elaborazione prima di installare o cambiare il percorso.",
    ],
    [
        "Use VB-CABLE",
        "Использовать VB-CABLE",
        "Utiliser VB-CABLE",
        "VB-CABLE verwenden",
        "Usar VB-CABLE",
        "Usa VB-CABLE",
    ],
    [
        "After restart: Refresh devices, Use VB-CABLE, select your browser in Audio, then Start processing.",
        "После перезагрузки: обновите устройства, выберите VB-CABLE, укажите браузер в разделе «Аудио» и запустите обработку.",
        "Après redémarrage : actualisez les appareils, utilisez VB-CABLE, choisissez le navigateur dans Audio, puis démarrez le traitement.",
        "Nach dem Neustart: Geräte aktualisieren, VB-CABLE verwenden, Browser unter Audio wählen und Verarbeitung starten.",
        "Tras reiniciar: actualiza dispositivos, usa VB-CABLE, elige el navegador en Audio e inicia el procesamiento.",
        "Dopo il riavvio: aggiorna i dispositivi, usa VB-CABLE, scegli il browser in Audio e avvia l'elaborazione.",
    ],
    [
        "Installed:",
        "Установлено:",
        "Installé :",
        "Installiert:",
        "Instalado:",
        "Installato:",
    ],
    [
        "Available:",
        "Доступно:",
        "Disponible :",
        "Verfügbar:",
        "Disponible:",
        "Disponibile:",
    ],
    [
        "Check for updates",
        "Проверить обновления",
        "Rechercher des mises à jour",
        "Nach Updates suchen",
        "Buscar actualizaciones",
        "Cerca aggiornamenti",
    ],
    [
        "All releases",
        "Все версии",
        "Toutes les versions",
        "Alle Versionen",
        "Todas las versiones",
        "Tutte le versioni",
    ],
    [
        "Check for updates on startup",
        "Проверять обновления при запуске",
        "Rechercher des mises à jour au démarrage",
        "Beim Start nach Updates suchen",
        "Buscar actualizaciones al iniciar",
        "Cerca aggiornamenti all'avvio",
    ],
    [
        "Release notes",
        "Описание версии",
        "Notes de version",
        "Versionshinweise",
        "Notas de la versión",
        "Note di rilascio",
    ],
    [
        "Install later",
        "Установить позже",
        "Installer plus tard",
        "Später installieren",
        "Instalar después",
        "Installa più tardi",
    ],
    [
        "Restart & install",
        "Перезапустить и установить",
        "Redémarrer et installer",
        "Neustarten und installieren",
        "Reiniciar e instalar",
        "Riavvia e installa",
    ],
    [
        "Download and install",
        "Скачать и установить",
        "Télécharger et installer",
        "Herunterladen und installieren",
        "Descargar e instalar",
        "Scarica e installa",
    ],
    [
        "After verification: save settings, stop audio and restart.",
        "После проверки: сохранение настроек, остановка звука и перезапуск.",
        "Après vérification : enregistrer les paramètres, arrêter l'audio et redémarrer.",
        "Nach Prüfung: Einstellungen speichern, Audio stoppen und neu starten.",
        "Tras verificar: guardar ajustes, detener el audio y reiniciar.",
        "Dopo la verifica: salva le impostazioni, arresta l'audio e riavvia.",
    ],
    [
        "Restart saves settings and stops processing.",
        "Перезапуск сохраняет настройки и останавливает обработку.",
        "Le redémarrage enregistre les paramètres et arrête le traitement.",
        "Ein Neustart speichert die Einstellungen und stoppt die Verarbeitung.",
        "Reiniciar guarda los ajustes y detiene el procesamiento.",
        "Il riavvio salva le impostazioni e arresta l'elaborazione.",
    ],
    [
        "Devices, optional driver and application updates.",
        "Устройства, дополнительный драйвер и обновления приложения.",
        "Appareils, pilote facultatif et mises à jour de l'application.",
        "Geräte, optionaler Treiber und Anwendungsupdates.",
        "Dispositivos, controlador opcional y actualizaciones.",
        "Dispositivi, driver facoltativo e aggiornamenti.",
    ],
    ["Audio", "Аудио", "Audio", "Audio", "Audio", "Audio"],
    [
        "Driver (optional)",
        "Драйвер (необязательно)",
        "Pilote (facultatif)",
        "Treiber (optional)",
        "Controlador (opcional)",
        "Driver (facoltativo)",
    ],
    [
        "Updates",
        "Обновления",
        "Mises à jour",
        "Updates",
        "Actualizaciones",
        "Aggiornamenti",
    ],
    [
        "Preview · no devices or settings are changed",
        "Предпросмотр · устройства и настройки не меняются",
        "Aperçu · aucun appareil ni paramètre modifié",
        "Vorschau · keine Geräte oder Einstellungen geändert",
        "Vista previa · no se cambian dispositivos ni ajustes",
        "Anteprima · dispositivi e impostazioni invariati",
    ],
    ["Quit", "Выйти", "Quitter", "Beenden", "Salir", "Esci"],
    [
        "Quit Sound Lock",
        "Выйти из Sound Lock",
        "Quitter Sound Lock",
        "Sound Lock beenden",
        "Salir de Sound Lock",
        "Esci da Sound Lock",
    ],
    [
        "Needs attention",
        "Требуется внимание",
        "Action nécessaire",
        "Handlungsbedarf",
        "Requiere atención",
        "Richiede attenzione",
    ],
    [
        "Copy details",
        "Скопировать сведения",
        "Copier les détails",
        "Details kopieren",
        "Copiar detalles",
        "Copia dettagli",
    ],
    [
        "Close",
        "Закрыть",
        "Fermer",
        "Schließen",
        "Cerrar",
        "Chiudi",
    ],
    [
        "Details",
        "Подробнее",
        "Détails",
        "Details",
        "Detalles",
        "Dettagli",
    ],
    [
        "Audio needs attention",
        "Проверьте аудио",
        "Vérifiez l'audio",
        "Audio prüfen",
        "Revisa el audio",
        "Controlla l'audio",
    ],
    [
        "Processing",
        "Обработка",
        "Traitement",
        "Verarbeitung",
        "Procesando",
        "Elaborazione",
    ],
    ["Ready", "Готово", "Prêt", "Bereit", "Listo", "Pronto"],
    ["Stop", "Стоп", "Arrêter", "Stoppen", "Detener", "Arresta"],
    ["Start", "Старт", "Démarrer", "Starten", "Iniciar", "Avvia"],
    [
        "Set up audio",
        "Настроить звук",
        "Configurer l'audio",
        "Audio einrichten",
        "Configurar audio",
        "Configura audio",
    ],
    [
        "Start processing",
        "Запустить обработку",
        "Démarrer le traitement",
        "Verarbeitung starten",
        "Iniciar procesamiento",
        "Avvia elaborazione",
    ],
    [
        "Stop processing",
        "Остановить обработку",
        "Arrêter le traitement",
        "Verarbeitung stoppen",
        "Detener procesamiento",
        "Arresta elaborazione",
    ],
    [
        "Leveling enabled",
        "Выравнивание включено",
        "Égalisation activée",
        "Pegelausgleich aktiviert",
        "Nivelación activada",
        "Livellamento attivo",
    ],
    [
        "Sound Lock · steady sound",
        "Sound Lock · ровный звук",
        "Sound Lock · son régulier",
        "Sound Lock · gleichmäßiger Klang",
        "Sound Lock · sonido estable",
        "Sound Lock · suono uniforme",
    ],
    [
        "Update available: v{version}",
        "Доступно обновление: v{version}",
        "Mise à jour disponible : v{version}",
        "Update verfügbar: v{version}",
        "Actualización disponible: v{version}",
        "Aggiornamento disponibile: v{version}",
    ],
    [
        "Sound Lock · update v{version} available",
        "Sound Lock · доступна версия v{version}",
        "Sound Lock · mise à jour v{version} disponible",
        "Sound Lock · Update v{version} verfügbar",
        "Sound Lock · actualización v{version} disponible",
        "Sound Lock · aggiornamento v{version} disponibile",
    ],
    [
        "Waiting for {browser} audio",
        "Ожидание звука {browser}",
        "En attente de l'audio de {browser}",
        "Warten auf Audio von {browser}",
        "Esperando audio de {browser}",
        "In attesa dell'audio di {browser}",
    ],
    [
        "{browser} routed to Source",
        "{browser} направлен в источник",
        "{browser} dirigé vers Source",
        "{browser} der Quelle zugewiesen",
        "{browser} dirigido a Fuente",
        "{browser} inviato alla sorgente",
    ],
    [
        "Download and installation are separate steps.",
        "Загрузка и установка выполняются отдельно.",
        "Le téléchargement et l'installation sont séparés.",
        "Download und Installation sind getrennte Schritte.",
        "La descarga y la instalación son pasos separados.",
        "Download e installazione sono passaggi separati.",
    ],
    [
        "Downloading the official package…",
        "Загрузка официального пакета…",
        "Téléchargement du paquet officiel…",
        "Offizielles Paket wird heruntergeladen…",
        "Descargando el paquete oficial…",
        "Download del pacchetto ufficiale…",
    ],
    [
        "Opening the installer · approve Windows UAC to continue.",
        "Открытие установщика · подтвердите запрос UAC.",
        "Ouverture de l'installateur · acceptez l'UAC pour continuer.",
        "Installationsprogramm wird geöffnet · Windows-UAC zum Fortfahren bestätigen.",
        "Abriendo el instalador · acepta UAC para continuar.",
        "Apertura del programma di installazione · approva UAC per continuare.",
    ],
    [
        "Download verified. Click Install driver when ready.",
        "Загрузка проверена. Нажмите «Установить драйвер», когда будете готовы.",
        "Téléchargement vérifié. Cliquez sur Installer le pilote quand vous êtes prêt.",
        "Download geprüft. Bei Bereitschaft auf Treiber installieren klicken.",
        "Descarga verificada. Pulsa Instalar controlador cuando estés listo.",
        "Download verificato. Premi Installa driver quando sei pronto.",
    ],
    [
        "Installer closed. Restart Windows, then refresh devices.",
        "Установщик закрыт. Перезагрузите Windows и обновите устройства.",
        "Installateur fermé. Redémarrez Windows, puis actualisez les appareils.",
        "Installationsprogramm geschlossen. Windows neu starten, dann Geräte aktualisieren.",
        "Instalador cerrado. Reinicia Windows y actualiza los dispositivos.",
        "Programma di installazione chiuso. Riavvia Windows e aggiorna i dispositivi.",
    ],
    [
        "Setup could not finish. Retry or use the official website.",
        "Не удалось завершить установку. Повторите или используйте официальный сайт.",
        "Installation inachevée. Réessayez ou utilisez le site officiel.",
        "Installation nicht abgeschlossen. Erneut versuchen oder offizielle Website verwenden.",
        "No se pudo completar. Reintenta o usa el sitio oficial.",
        "Impossibile completare l'installazione. Riprova o usa il sito ufficiale.",
    ],
    [
        "Check GitHub for a newer version.",
        "Проверьте наличие новой версии на GitHub.",
        "Recherchez une nouvelle version sur GitHub.",
        "Auf GitHub nach einer neueren Version suchen.",
        "Busca una versión más reciente en GitHub.",
        "Cerca una versione più recente su GitHub.",
    ],
    [
        "Checking GitHub…",
        "Проверка GitHub…",
        "Vérification sur GitHub…",
        "GitHub wird geprüft…",
        "Consultando GitHub…",
        "Controllo su GitHub…",
    ],
    [
        "Downloading and verifying the update…",
        "Загрузка и проверка обновления…",
        "Téléchargement et vérification de la mise à jour…",
        "Update wird heruntergeladen und geprüft…",
        "Descargando y verificando la actualización…",
        "Download e verifica dell'aggiornamento…",
    ],
    [
        "A newer version is available.",
        "Доступна новая версия.",
        "Une nouvelle version est disponible.",
        "Eine neuere Version ist verfügbar.",
        "Hay una nueva versión disponible.",
        "È disponibile una nuova versione.",
    ],
    [
        "You're up to date.",
        "Установлена последняя версия.",
        "Vous disposez de la dernière version.",
        "Die Version ist aktuell.",
        "Tienes la última versión.",
        "La versione è aggiornata.",
    ],
    [
        "Verified update ready. Restart when convenient.",
        "Проверенное обновление готово. Перезапустите в удобное время.",
        "Mise à jour vérifiée. Redémarrez quand cela vous convient.",
        "Geprüftes Update bereit. Bei Gelegenheit neu starten.",
        "Actualización verificada. Reinicia cuando te convenga.",
        "Aggiornamento verificato e pronto. Riavvia quando preferisci.",
    ],
    [
        "Update ready. Close driver setup, then restart to install.",
        "Обновление готово. Закройте установщик драйвера и перезапустите приложение.",
        "Mise à jour prête. Fermez l'installation du pilote, puis redémarrez.",
        "Update bereit. Treiberinstallation schließen, dann zur Installation neu starten.",
        "Actualización lista. Cierra el instalador del controlador y reinicia.",
        "Aggiornamento pronto. Chiudi l'installazione del driver e riavvia.",
    ],
    [
        "The previous version was restored. Review the error or retry.",
        "Предыдущая версия восстановлена. Проверьте ошибку или повторите.",
        "La version précédente a été restaurée. Consultez l'erreur ou réessayez.",
        "Vorherige Version wiederhergestellt. Fehler prüfen oder erneut versuchen.",
        "Se restauró la versión anterior. Revisa el error o reintenta.",
        "La versione precedente è stata ripristinata. Controlla l'errore o riprova.",
    ],
    [
        "Audio device unavailable. Reconnect it, open Settings and refresh devices.",
        "Аудиоустройство недоступно. Подключите его снова и обновите устройства в настройках.",
        "Appareil audio indisponible. Reconnectez-le, puis actualisez les appareils dans Paramètres.",
        "Audiogerät nicht verfügbar. Wieder anschließen und Geräte in den Einstellungen aktualisieren.",
        "Dispositivo de audio no disponible. Reconéctalo y actualiza los dispositivos en Ajustes.",
        "Dispositivo audio non disponibile. Ricollegalo e aggiorna i dispositivi nelle impostazioni.",
    ],
    [
        "Tray unavailable",
        "Панель уведомлений недоступна",
        "Zone de notification indisponible",
        "Infobereich nicht verfügbar",
        "Bandeja no disponible",
        "Area di notifica non disponibile",
    ],
    [
        "Could not enumerate devices",
        "Не удалось получить список устройств",
        "Impossible de lister les appareils",
        "Geräte konnten nicht aufgelistet werden",
        "No se pudieron listar los dispositivos",
        "Impossibile elencare i dispositivi",
    ],
    [
        "Could not read settings",
        "Не удалось прочитать настройки",
        "Impossible de lire les paramètres",
        "Einstellungen konnten nicht gelesen werden",
        "No se pudieron leer los ajustes",
        "Impossibile leggere le impostazioni",
    ],
    [
        "Could not save settings",
        "Не удалось сохранить настройки",
        "Impossible d'enregistrer les paramètres",
        "Einstellungen konnten nicht gespeichert werden",
        "No se pudieron guardar los ajustes",
        "Impossibile salvare le impostazioni",
    ],
    [
        "Could not save audio settings before setup",
        "Не удалось сохранить настройки аудио перед установкой",
        "Impossible d'enregistrer les réglages audio avant l'installation",
        "Audioeinstellungen konnten vor der Installation nicht gespeichert werden",
        "No se pudieron guardar los ajustes de audio antes de instalar",
        "Impossibile salvare le impostazioni audio prima dell'installazione",
    ],
    [
        "Could not confirm update startup",
        "Не удалось подтвердить запуск обновления",
        "Impossible de confirmer le démarrage de la mise à jour",
        "Update-Start konnte nicht bestätigt werden",
        "No se pudo confirmar el inicio de la actualización",
        "Impossibile confermare l'avvio dell'aggiornamento",
    ],
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name))
            .collect()
    }
    #[test]
    fn catalogs_are_complete_unique_and_preserve_parameters() {
        let mut keys = BTreeSet::new();
        for row in MESSAGES {
            assert!(keys.insert(row[0]), "duplicate message: {}", row[0]);
            for text in row {
                assert!(!text.trim().is_empty(), "empty translation: {}", row[0]);
                assert_eq!(
                    placeholders(row[0]),
                    placeholders(text),
                    "{}: {text}",
                    row[0]
                );
            }
        }
    }
    #[test]
    fn unknown_text_is_preserved_and_languages_are_distinct() {
        for language in Language::ALL {
            assert_eq!(
                language.text("My custom headphones"),
                "My custom headphones"
            );
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
        assert_eq!(Language::Russian.text("Settings"), "Настройки");
        assert_eq!(Language::French.text("Theme"), "Thème");
    }
}
