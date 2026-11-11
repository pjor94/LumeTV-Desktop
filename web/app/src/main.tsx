import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { DesktopApp } from './components/Startup';
import { bootstrap } from './app';
import { motion } from './motion';
import '../app.css';
import '../motion.css';
import './widevine.css';
import './startup.css';

motion.initialize(document);
createRoot(document.getElementById('app')!).render(
    <StrictMode>
        <DesktopApp />
    </StrictMode>,
);
void bootstrap();
