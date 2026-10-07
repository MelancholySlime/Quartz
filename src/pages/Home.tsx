import { useMemo } from 'react';
import {
    ArrowLeftRight, Brush, Code, Dices, FileDigit, FolderInput, FolderSearch,
    Github, Image, Maximize, Music, Pipette, Settings, Sparkles,
    Waypoints, Wrench, type LucideIcon,
} from 'lucide-react';
import { useNavigationStore, type Page } from '@/lib/stores';
import { useTranslation, type TranslationKey } from '@/i18n';

interface ToolCardDef {
    id: string;
    titleKey: TranslationKey;
    descKey: TranslationKey;
    defaultTitle: string;
    defaultDesc: string;
    icon: LucideIcon;
    page: Page;
    isNew?: boolean;
}

const TOOL_CARD_DEFS: ToolCardDef[] = [
    { id: 'paint', titleKey: 'home.cards.paintTitle', descKey: 'home.cards.paintDesc', defaultTitle: 'Paint', defaultDesc: 'Customize your particles with ease. Choose from Random Colors, apply a Hue Shift, or generate a range of Shades.', icon: Brush, page: 'paint' },
    { id: 'port', titleKey: 'home.cards.portTitle', descKey: 'home.cards.portDesc', defaultTitle: 'Port', defaultDesc: 'Bring particles from different champions or skins into your own custom skin!', icon: ArrowLeftRight, page: 'port' },
    { id: 'vfxhub', titleKey: 'home.cards.vfxHubTitle', descKey: 'home.cards.vfxHubDesc', defaultTitle: 'VFX Hub', defaultDesc: 'Community-powered VFX sharing exclusively for Divine members.', icon: Github, page: 'port' },
    { id: 'wadexplorer', titleKey: 'home.cards.wadExplorerTitle', descKey: 'home.cards.wadExplorerDesc', defaultTitle: 'WAD Explorer', defaultDesc: 'Advanced explorer for WAD files with live 3D model and texture preview.', icon: FolderSearch, page: 'wadexplorer', isNew: true },

    { id: 'imgrecolor', titleKey: 'home.cards.imgRecolorTitle', descKey: 'home.cards.imgRecolorDesc', defaultTitle: 'Image Recolor', defaultDesc: 'Automatically batch recolor DDS or TEX files by simply selecting a folder and clicking "Batch Apply".', icon: Image, page: 'imgrecolor' },
    { id: 'bineditor', titleKey: 'home.cards.binEditorTitle', descKey: 'home.cards.binEditorDesc', defaultTitle: 'Bin Editor', defaultDesc: 'Primarily designed for editing parameters like birthscale directly within Quartz.', icon: Code, page: 'bineditor' },
    { id: 'assetextractor', titleKey: 'home.cards.assetExtractorTitle', descKey: 'home.cards.assetExtractorDesc', defaultTitle: 'Asset Extractor', defaultDesc: 'Extract and decompose League of Legends game assets from WAD files.', icon: FolderInput, page: 'assetextractor' },
    { id: 'soundbanks', titleKey: 'home.cards.soundBanksTitle', descKey: 'home.cards.soundBanksDesc', defaultTitle: 'Sound Banks', defaultDesc: 'Extract, edit, and repack audio bank files for custom sound mods.', icon: Music, page: 'soundbanks' },

    { id: 'upscale', titleKey: 'home.cards.upscaleTitle', descKey: 'home.cards.upscaleDesc', defaultTitle: 'Upscale', defaultDesc: 'AI-powered image upscaling for DDS and PNG texture files.', icon: Maximize, page: 'upscale' },
    { id: 'fakegear', titleKey: 'home.cards.fakeGearTitle', descKey: 'home.cards.fakeGearDesc', defaultTitle: 'FakeGear', defaultDesc: 'Enables a Ctrl+5 in-game toggle to swap between VFX variants on your custom skin.', icon: Sparkles, page: 'fakegear' },
    { id: 'randomizer', titleKey: 'home.cards.randomizerTitle', descKey: 'home.cards.randomizerDesc', defaultTitle: 'Randomizer', defaultDesc: 'Randomize VFX particle parameters across your entire skin at once.', icon: Dices, page: 'particlerandomizer' },
    { id: 'rgba', titleKey: 'home.cards.rgbaTitle', descKey: 'home.cards.rgbaDesc', defaultTitle: 'RGBA', defaultDesc: 'Adjust RGBA color channels on DDS and TEX texture files.', icon: Pipette, page: 'rgba' },

    { id: 'bumpath', titleKey: 'home.cards.bumpathTitle', descKey: 'home.cards.bumpathDesc', defaultTitle: 'Bumpath', defaultDesc: 'Repath League of Legends file references across your skin files.', icon: Waypoints, page: 'bumpath' },
    { id: 'filehandler', titleKey: 'home.cards.fileHandlerTitle', descKey: 'home.cards.fileHandlerDesc', defaultTitle: 'File Handler', defaultDesc: 'Universal file processing and randomization utility for bulk operations.', icon: FileDigit, page: 'filehandler' },
    { id: 'tools', titleKey: 'home.cards.toolsTitle', descKey: 'home.cards.toolsDesc', defaultTitle: 'Tools', defaultDesc: 'Add your own executables and drag-and-drop them with your folder to apply the fixes.', icon: Wrench, page: 'tools' },
    { id: 'settings', titleKey: 'home.cards.settingsTitle', descKey: 'home.cards.settingsDesc', defaultTitle: 'Settings', defaultDesc: 'Select your preferred font and configure the Ritobin CLI path.', icon: Settings, page: 'settings' },
];

function tourKey(title: string) {
    return `card-${title.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
}

function Home() {
    const { t } = useTranslation();
    const setPage = useNavigationStore((state) => state.setPage);

    const cards = useMemo(() => {
        return TOOL_CARD_DEFS.map((def) => ({
            ...def,
            title: t(def.titleKey) || def.defaultTitle,
            description: t(def.descKey) || def.defaultDesc,
        }));
    }, [t]);

    return (
        <div className="main-page-container">
            <section className="main-hero">
                <div className="main-hero__glow main-hero__glow--primary" />
                <div className="main-hero__glow main-hero__glow--secondary" />
                <h1>Quartz</h1>
                <p>{t('home.subtitle') || 'League of Legends Toolkit'}</p>
            </section>

            <div className="main-separator" />

            <section className="main-tool-grid">
                {cards.map((tool) => {
                    const Icon = tool.icon;
                    return (
                        <button
                            key={tool.id}
                            type="button"
                            className={`main-page-card${tool.isNew ? ' is-new' : ''}`}
                            data-tour={tourKey(tool.defaultTitle)}
                            title={`${tool.title}\n${tool.description}`}
                            onClick={() => setPage(tool.page)}
                        >
                            {tool.isNew && <span className="main-page-card__badge">{t('common.new')}</span>}
                            <span className="main-page-card__head">
                                <Icon size={18} />
                                <strong>{tool.title}</strong>
                            </span>
                            <span className="main-page-card__desc">{tool.description}</span>
                        </button>
                    );
                })}
            </section>
        </div>
    );
}

export { Home };
export default Home;

