import { createContext, useContext } from 'react';
import type { PanelManager } from '../hooks/usePanelManager';
import type { HeaderStage } from '../hooks/useHeaderBreakpoints';

interface SidebarContextValue {
  manager: PanelManager;
  headerStage: HeaderStage;
}

const noopManager: PanelManager = {
  isCollapsed: () => false,
  toggle: () => {},
  expand: () => {},
  autoResize: () => {},
  getWidth: () => 0,
  setWidth: () => {},
  onDragEnd: () => {},
  collapseWithInfinity: () => {},
  collapsedState: {},
  getDebugInfo: () => ({ lastWidth: 0, userThresholds: new Map(), widths: {} }),
};

export const SidebarContext = createContext<SidebarContextValue>({
  manager: noopManager,
  headerStage: 'full',
});

export const useSidebar = () => useContext(SidebarContext);
